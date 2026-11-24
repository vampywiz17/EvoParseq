# EvoParseq privacy policy

This policy covers the EvoParseq client for Windows, Linux and Android TV and
information you choose to share with the project. EvoParseq is maintained
independently of Parsec and Phaze. The project does not operate an account,
analytics or remote session service for the client.

## Account and host access

When you sign in, EvoParseq sends your email address, password and any
two-factor code you enter to the service you selected. The client uses that
service's session token to authenticate subsequent requests such as listing
available computers and connecting to the host you select. The service handles
the account under its own privacy policy.

- [Parsec privacy policy](https://parsec.app/privacy)
- [Phaze privacy policy](https://phaze.app/privacy/)

## Connections and third-party services

The client exchanges connection information, such as network addresses and
session identifiers, with the selected service, host and the connection
servers used by your configuration. STUN helps determine how endpoints can
reach each other. The default Parsec STUN endpoint is operated by Parsec.
With the Cloudflare provider in STUN-only mode, the client can contact
Cloudflare's STUN server without a TURN key. If you configure custom STUN
servers, the client sends requests to those servers. The operator of a STUN
server can see the source IP address of the request. If a relay is used, it
carries session traffic and its operator can see connection metadata such as
IP addresses and traffic volumes.

A session carries video and audio from the host and sends your input to the
host. Clipboard or USB HID data is exchanged when those features are in use.
The EvoParseq project does not receive this session traffic.

If you configure Cloudflare TURN, the client sends your TURN key identifier
and API token to Cloudflare to obtain temporary relay credentials. For a
custom relay, the client uses the server and credentials you configured.
GitHub receives requests made by the client to check for or download updates.
GitHub also hosts project downloads and support discussions.

- [Cloudflare privacy policy](https://www.cloudflare.com/privacypolicy/)
- [GitHub privacy statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement)

## Local storage and diagnostics

The client stores preferences locally. If you enable remembered sign-in, the
client keeps a protected session token on the device. The desktop client can
also keep the sign-in email. The client does not save account passwords or
two-factor codes. Remembered credentials use Windows data protection, the
Linux desktop Secret portal or Android Keystore, depending on the platform.

When you configure Cloudflare TURN or a custom TURN relay on desktop, the
client saves the TURN key identifier or relay username in its connection
settings. It saves the API token or relay password using Windows data
protection or the Linux desktop Secret portal. Signing out leaves these
connection settings and credentials in place. To remove a saved secret,
select **Forget saved API token** or **Forget saved password** in the
connection settings and save the change. If you enable credential caching,
temporary Cloudflare TURN credentials are cached in memory.

Optional diagnostic logging stores operational information locally, such as
application version, event timestamps, connection state and media diagnostics.
The client does not automatically upload these logs. You can turn off
diagnostic logging and remove existing log files. Before sharing a log, review
it for identifiers and other information about your setup.

Sign out to remove the remembered session from the client. To remove other
local settings or logs, delete the relevant profile files on desktop or
clear the app's data on Android TV. For data held by an account or relay
provider, use that provider's privacy controls.

## Support and privacy questions

Information you choose to share with the project is used to respond to your
request and investigate reported issues. GitHub issues and their attachments
are public. For a privacy question about information you shared with the
project, contact the [project maintainer](https://github.com/vampywiz17)
through the contact information on the maintainer profile. You can also open
an issue requesting a private response without posting personal information.
