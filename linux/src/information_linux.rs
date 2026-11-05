//! Native information windows and explicit system-installer handoff.
use crate::{Desktop, about, release_flatpak};
use anyhow::{Context as _, Result, ensure};
use eframe::egui;
use std::{path::PathBuf, sync::mpsc};
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    About,
    Updates,
}
#[derive(Default)]
pub struct State {
    pub mode: Option<Mode>,
    candidate: Option<release_flatpak::Candidate>,
    message: String,
    pending: Option<mpsc::Receiver<Result<Outcome, String>>>,
}
enum Outcome {
    Checked(Option<release_flatpak::Candidate>),
    Opened,
}
impl State {
    fn run(&mut self, ctx: &egui::Context, job: impl FnOnce() -> Result<Outcome> + Send + 'static) {
        if self.pending.is_some() {
            return;
        }
        let (send, recv) = mpsc::channel();
        let context = ctx.clone();
        match std::thread::Builder::new()
            .name("release-update".into())
            .spawn(move || {
                let _ = send.send(job().map_err(|e| format!("{e:#}")));
                context.request_repaint();
            }) {
            Ok(_) => self.pending = Some(recv),
            Err(_) => self.message = "Could not start the update worker.".into(),
        }
    }
    pub fn open(&mut self, ctx: &egui::Context, mode: Mode) {
        self.mode = Some(mode);
        if mode == Mode::Updates && self.pending.is_none() {
            self.check(ctx);
        }
    }
    fn check(&mut self, ctx: &egui::Context) {
        self.candidate = None;
        self.message = "Checking GitHub for a newer Linux Flatpak...".into();
        self.run(ctx, || {
            Ok(Outcome::Checked(release_flatpak::check(env!(
                "CARGO_PKG_VERSION"
            ))?))
        });
    }
    fn receive(&mut self) {
        let Some(receiver) = &self.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(_) => Err("The update worker stopped.".into()),
        };
        self.pending = None;
        match result {
            Ok(Outcome::Checked(candidate))=>{
                self.message=if candidate.is_some() {"A newer Linux package is available. Review the notes, then open the system installer."} else {"No newer compatible Linux Flatpak is published on GitHub."}.into();
                self.candidate=candidate;
            }
            Ok(Outcome::Opened)=>self.message="The request was handed to the desktop. Confirm installation there, then close and restart EvoParseq after it finishes.".into(),
            Err(message)=>self.message=message,
        }
    }
}
fn runtime<T>(job: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(120), job)
                .await
                .context("Desktop request timed out")?
        })
}
fn project_page() -> Result<Outcome> {
    runtime(async {
        ashpd::desktop::open_uri::OpenFileRequest::default()
            .send_uri(&ashpd::Uri::parse(about::REPOSITORY)?)
            .await?
            .response()?;
        Ok(Outcome::Opened)
    })
}
fn cache_directory() -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
        .context("No private cache directory")?;
    let app = base.join("EvoParseq");
    std::fs::create_dir_all(&app)?;
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o700))?;
    Ok(app)
}
fn install(candidate: release_flatpak::Candidate) -> Result<Outcome> {
    use std::os::unix::fs::OpenOptionsExt;
    let directory = cache_directory()?;
    let path = directory.join(candidate.filename());
    let open = || {
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
    };
    let mut file = match open() {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut random = [0u8; 16];
            getrandom::fill(&mut random)
                .map_err(|_| anyhow::anyhow!("Secure update filename unavailable"))?;
            let token = random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let temporary = directory.join(format!("{token}.flatpak"));
            candidate.download(&temporary)?;
            if let Err(error) = std::fs::rename(&temporary, &path) {
                let _ = std::fs::remove_file(&temporary);
                return Err(error.into());
            }
            open()?
        }
        Err(error) => return Err(error.into()),
    };
    ensure!(file.metadata()?.is_file(), "Update is not a regular file");
    candidate.verify(&file)?;
    use std::io::Seek;
    file.rewind()?;
    runtime(async {
        ashpd::desktop::open_uri::OpenFileRequest::default().ask(true).writeable(false).send_file(&file).await?.response()?;
        Ok(Outcome::Opened)
    }).context("Cannot open the system installer. Install the verified cached Flatpak with your software manager")
    // Keep the verified file: the system installer may read it asynchronously.
    // No host shell, D-Bus escape permission, silent replacement or restart.
}
impl State {
    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        root: &egui::Context,
        mode: Mode,
        logo: &egui::TextureHandle,
    ) {
        use crate::launcher::{color, label, rect_at};
        ui.style_mut().override_font_id = Some(egui::FontId::proportional(16.));
        let size = ui.max_rect().size();
        let rects = about::rectangles(size.x as i32, size.y as i32);
        let origin = ui.max_rect().min;
        let r = |i: usize| rect_at(origin, rects[i]);
        let migration = "Linux Flatpak updates use the desktop software installer. Settings and protected sign-in data stay in this application's private storage. A GitHub preview is identified explicitly. Close and restart EvoParseq after installation.";
        let info = about::content(env!("CARGO_PKG_VERSION"), "Linux x86_64", "", migration);
        let (version, status, notes) = if mode == Mode::About {
            (info.version, info.status, info.notes.replace("\r\n", "\n"))
        } else {
            let available = self
                .candidate
                .as_ref()
                .map(|c| {
                    format!(
                        "\nAvailable: {} ({})",
                        c.version,
                        if c.prerelease { "preview" } else { "stable" }
                    )
                })
                .unwrap_or_default();
            (
                format!("Installed: v{}{available}", env!("CARGO_PKG_VERSION")),
                self.message.clone(),
                self.candidate
                    .as_ref()
                    .map(|c| format!("{}\n\n{migration}", c.notes))
                    .unwrap_or_else(|| migration.into()),
            )
        };
        ui.put(r(0), egui::Image::new((logo.id(), egui::vec2(40., 40.))));
        label(
            ui,
            r(1),
            if mode == Mode::About {
                about::HEADING
            } else {
                about::UPDATES_HEADING
            },
            32.,
            color(crate::app_style::TEXT),
        );
        label(
            ui,
            r(2),
            &version.replace("\r\n", "\n"),
            16.,
            color(crate::app_style::TEXT),
        );
        label(ui, r(3), &status, 16., color(crate::app_style::TEXT));
        let notes_rect = r(4);
        ui.scope_builder(egui::UiBuilder::new().max_rect(notes_rect), |ui| {
            ui.set_clip_rect(notes_rect);
            egui::ScrollArea::vertical()
                .id_salt("information-notes")
                .show(ui, |ui| {
                    ui.add(egui::Label::new(&notes).selectable(true).wrap());
                });
        });
        let check = r(5);
        let download = r(6);
        let close = r(7);
        ui.scope_builder(egui::UiBuilder::new().max_rect(check), |ui| {
            let button = egui::Button::new(if mode == Mode::About {
                about::PROJECT
            } else {
                about::CHECK_AGAIN
            })
            .min_size(check.size());
            if ui.add_enabled(self.pending.is_none(), button).clicked() {
                if mode == Mode::About {
                    self.run(root, project_page);
                } else {
                    self.check(root);
                }
            }
        });
        if mode == Mode::Updates {
            ui.scope_builder(egui::UiBuilder::new().max_rect(download), |ui| {
                let button = egui::Button::new("Install Flatpak").min_size(download.size());
                if ui
                    .add_enabled(self.pending.is_none() && self.candidate.is_some(), button)
                    .clicked()
                {
                    let candidate = self.candidate.clone().unwrap();
                    self.message = "Downloading and verifying the Linux Flatpak...".into();
                    self.run(root, move || install(candidate));
                }
            });
        }
        if ui.put(close, egui::Button::new(about::CLOSE)).clicked() {
            self.mode = None;
        }
    }
}
impl Desktop {
    pub(super) fn information(&mut self, ctx: &egui::Context) {
        self.ui_state.information.receive();
        let Some(mode) = self.ui_state.information.mode else {
            return;
        };
        let title = if mode == Mode::About {
            about::TITLE
        } else {
            about::UPDATES_TITLE
        };
        let size = if mode == Mode::About {
            about::SIZE
        } else {
            about::UPDATES_SIZE
        };
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_decorations(true)
            .with_inner_size(size)
            .with_min_inner_size(size)
            .with_max_inner_size(size)
            .with_resizable(false)
            .with_maximize_button(false);
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("information-native"),
            builder,
            |child, _| {
                if child.input(|i| i.viewport().close_requested()) {
                    self.ui_state.information.mode = None;
                    return;
                }
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::new()
                            .fill(crate::launcher::color(crate::app_style::BACKGROUND)),
                    )
                    .show(child, |ui| {
                        self.ui_state
                            .information
                            .contents(ui, ctx, mode, &self.logo)
                    });
            },
        );
    }
}
