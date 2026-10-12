# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Developers inside an organisation who need virtual machines and containers for their work, and who request and manage their own without touching the underlying hypervisor directly. They are technical, but infrastructure is a means for them, not their job.

Two more audiences:

- **Instance admins**, who install Lemnos, connect and maintain the environments (addresses, service logins, certificates), create organisations and decide who is in them.
- **People evaluating Lemnos** for their own organisation. They arrive at the public landing page, signed out, and want to know what it is and how to run it.

## Product Purpose

Lemnos is a self-hosted, self-service console for compute. A developer signs in with the organisation's own identity, sees what is running, and powers their guests on and off, without needing an account on, or knowledge of, the hypervisor behind it.

## Positioning

A multi-provider control plane: one console over several infrastructure backends. Proxmox VE is the first provider, not the product. The same developer-facing experience is meant to hold whichever backend a workload runs on.

## Operating Context

- Self-hosted: whoever runs Lemnos deploys it on their own hardware. There is no hosted service, no pricing and no paywall.
- One installation serves several organisations that share the same clusters. A person can belong to more than one organisation and switches between them.
- A public landing page is shown to signed-out visitors; everything else is behind sign-in.
- Sign-in is local accounts, directory (LDAP), SAML single sign-on and passkeys, with optional two-factor codes. Which methods are offered is decided at runtime by a config file.
- Environments can be declared in a TOML config file and are synced to the database on start, or added through the UI.

## Capabilities and Constraints

Built today (`crates/lemnos-ui`):

- Sign in, sign up, sign out, SSO, passkey sign-in, two-factor challenge.
- Account page: two-factor setup (QR code, manual key) and passkey management.
- Proxmox: list, add and remove environments; per environment, nodes, guests (VMs and containers) and storage, with start, shut down, reboot and stop for guests.
- An unreachable environment is reported as a message on the page, not a broken page.

Technical constraints:

- Server-rendered with Topcoat (Rust) and Tailwind. State changes are plain form POSTs followed by a redirect.
- Prefer building in Topcoat over standalone JavaScript files. JavaScript stays minimal; today the only script is the passkey one.
- Both a dark and a light mode are required.

Access model:

- **Instance admin.** Connects environments, creates organisations, gives them environments and adds people. Admins are named in the config file or by environment variables, with an initial password; on a fresh installation the first account created becomes one.
- **Organisation roles.** Owner (manages the organisation's members), member (uses its machines), viewer (sees them, cannot change them).
- **Joining an organisation.** Signing up never creates one. A person joins through the sign-in provider they use (set per organisation in the config file) or is added by an admin or owner.
- **Whose machine it is.** An organisation is given an environment together with a tag, and sees the machines there that carry the tag. This is the current answer, not the final one; see `ideas.md` for resource pools and groups from the identity provider.

Open decisions:

- The final way to tell organisations' machines apart, and to map directory groups to organisations (`ideas.md`).
- Legal pages and documentation for the public site: no content exists yet.
- Which providers follow Proxmox, and when. Crates named `lemnos-vmware` and `lemnos-kedalion` exist in the workspace; their state is not confirmed here.
- Whether provisioning (creating guests), quotas or requests are part of the product.

## Brand Commitments

- The name **Lemnos**.
- The tagline **Forge your own cloud.**
- Lemnos belongs to the **hephaistos-rs** organisation, whose projects share a Greek theme. The name is meant to be used, not just worn: the interface draws on the real island of Lemnos and on Hephaistos, the smith god who landed there.

The current blue palette and DM Sans were carried over from an earlier project and are not binding.

## Evidence on Hand

- The working UI in `crates/lemnos-ui/src/app/` and its stylesheet `crates/lemnos-ui/src/styles/app.css`.
- No logo, illustrations, screenshots, customer names, usage figures or testimonials exist. None may be invented.

## Product Principles

1. The provider is a detail. A developer manages a guest, not a Proxmox; provider-specific words appear only where they help.
2. Self-service means no help needed. Every state, including failure, says what happened and what to do next in plain language.
3. Actions on running machines are deliberate. Power changes are always explicit, always confirmable by their result, never triggered by navigation.
4. Works without scripts. The page a form returns is the truth; JavaScript only adds to it.
