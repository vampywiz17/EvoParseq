//! GitHub Flatpak asset policy. Linux owns private storage and installation UI.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{io::{Read, Write}, path::Path};
const REPOSITORY: &str = crate::about::REPOSITORY;
const API: &str = "https://api.github.com/repos/vampywiz17/EvoParseq/releases?per_page=100";
const LIMIT: u64 = 256*1024*1024;
#[derive(Clone, Debug)]
pub struct Candidate {
    pub version: String,
    pub notes: String,
    pub prerelease: bool,
    url: String,
    size: u64,
    digest: String,
}
#[derive(Deserialize)]
struct Release { tag_name: String, draft: bool, prerelease: bool, body: Option<String>, assets: Vec<Asset> }
#[derive(Deserialize)]
struct Asset { name: String, browser_download_url: String, size: u64, digest: Option<String> }
fn version(text: &str) -> Result<[u64;3]> {
    let parts:Vec<_>=text.split('.').collect();
    ensure!(parts.len()==3,"Invalid Linux package version");
    let mut out=[0;3];
    for (i,s) in parts.iter().enumerate() {
        ensure!(!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s.len()==1 || !s.starts_with('0')),"Invalid Linux package version");
        out[i]=s.parse()?;
    }
    Ok(out)
}
fn select(bytes: &[u8], installed: &str, arch: &str) -> Result<Option<Candidate>> {
    let installed=version(installed)?;
    let releases:Vec<Release>=serde_json::from_slice(bytes)?;
    let mut best:Option<([u64;3],Candidate)>=None;
    for release in releases {
        if release.draft { continue; }
        // Package versions are independent from Windows/Android release tags.
        // Linux previews remain explicitly labelled, never offered as stable.
        ensure!(release.tag_name.len() <= 200,"Invalid release tag");
        if !release.tag_name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) { continue; }
        for asset in release.assets {
            let Some(text)=asset.name.strip_prefix("EvoParseq-Linux-v").and_then(|n| n.strip_suffix(&format!("-{arch}.flatpak"))) else { continue; };
            let Ok(key)=version(text) else { continue; };
            if key <= installed { continue; }
            ensure!(asset.browser_download_url==format!("{REPOSITORY}/releases/download/{}/{}",release.tag_name,asset.name),"Untrusted Flatpak URL");
            ensure!(asset.size>0 && asset.size<=LIMIT,"Invalid Flatpak size");
            let digest=asset.digest.context("Flatpak asset has no SHA-256 digest")?;
            crate::release_http::hash(&digest)?;
            if let Some((old,candidate))=&best {
                if *old==key {
                    // One version must identify one immutable package.
                    ensure!(candidate.digest==digest,"Ambiguous Flatpak package version");
                }
                if *old>key || (*old==key && !candidate.prerelease) { continue; }
            }
            best=Some((key,Candidate{version:text.into(),notes:release.body.as_deref().unwrap_or_default().chars().take(12000).collect(),prerelease:release.prerelease,url:asset.browser_download_url,size:asset.size,digest}));
        }
    }
    Ok(best.map(|(_,c)|c))
}
pub fn check(installed: &str) -> Result<Option<Candidate>> {
    select(&crate::release_http::metadata(API)?,installed,std::env::consts::ARCH)
}
impl Candidate {
    pub fn filename(&self) -> String {
        format!("EvoParseq-Linux-v{}-{}.flatpak",self.version,std::env::consts::ARCH)
    }
    pub fn download(&self, destination: &Path) -> Result<()> {
        // Caller supplies an app-private cache. Never overwrite an existing file
        // or follow a download symlink; a cancelled/failed transfer is removed.
        let mut options=std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        let mut file=options.open(destination)?;
        let result=(|| -> Result<()> {
            let response=crate::release_http::client()?.get(&self.url).send()?.error_for_status()?;
            if let Some(size)=response.content_length() { ensure!(size==self.size,"Flatpak size changed"); }
            crate::release_http::copy_verified(response,&mut file,self.size,&self.digest)?;
            file.flush()?; file.sync_all()?;
            Ok(())
        })();
        drop(file);
        if result.is_err() { let _=std::fs::remove_file(destination); }
        result
    }
    pub fn verify(&self, file: impl Read) -> Result<()> {
        crate::release_http::copy_verified(file,&mut std::io::sink(),self.size,&self.digest)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn release(package: &str, pre: bool) -> serde_json::Value {
        let name=format!("EvoParseq-Linux-v{package}-x86_64.flatpak");
        serde_json::json!({"tag_name":"v0.9.6-dev.2","draft":false,"prerelease":pre,"body":"Changes","assets":[{"name":name,"size":12,"digest":format!("sha256:{}","ab".repeat(32)),"browser_download_url":format!("{REPOSITORY}/releases/download/v0.9.6-dev.2/{name}")}]})
    }
    fn parse(data:serde_json::Value)->Result<Option<Candidate>> {select(&serde_json::to_vec(&data)?,"0.1.16","x86_64")}
    #[test]
    fn linux_update_uses_package_version_architecture_and_explicit_preview() {
        let candidate=parse(serde_json::json!([release("0.1.17",true),release("0.1.18",false)])).unwrap().unwrap();
        assert_eq!(candidate.version,"0.1.18"); assert!(!candidate.prerelease);
        assert!(parse(serde_json::json!([release("0.1.16",false)])).unwrap().is_none());
        assert!(select(&serde_json::to_vec(&vec![release("0.1.18",false)]).unwrap(),"0.1.16","aarch64").unwrap().is_none());
        for bad in ["1.2","01.2.3","1.2.3/evil","1.2.3-dev.1"] { assert!(version(bad).is_err()); }
    }
    #[test]
    fn linux_update_rejects_redirect_injection_missing_digest_and_ambiguous_packages() {
        for field in ["browser_download_url","digest","size"] {
            let mut value=release("0.1.17",true);
            value["assets"][0][field]=if field=="size" {serde_json::json!(LIMIT+1)} else {serde_json::json!("bad")};
            assert!(parse(serde_json::json!([value])).is_err());
        }
        let mut other=release("0.1.17",true);other["assets"][0]["digest"]=serde_json::json!(format!("sha256:{}","cd".repeat(32)));
        assert!(parse(serde_json::json!([release("0.1.17",true),other])).is_err());
    }
    #[test]
    fn linux_update_requires_exact_download_size_and_sha256() {
        use sha2::{Digest,Sha256};
        let bytes=b"isolated Flatpak fixture";
        let c=Candidate{version:"0.1.17".into(),notes:String::new(),prerelease:true,url:String::new(),size:bytes.len() as u64,digest:format!("sha256:{:x}",Sha256::digest(bytes))};
        assert!(c.verify(&bytes[..]).is_ok());
        assert!(c.verify(&bytes[..bytes.len()-1]).is_err());
        assert!(c.verify(&[0u8;100][..]).is_err());
    }
}
