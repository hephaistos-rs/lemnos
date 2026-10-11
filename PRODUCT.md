# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Developers inside an organisation who need virtual machines and containers for their work, and who request and manage their own without touching the underlying hypervisor directly. They are technical, but infrastructure is a means for them, not their job.

A second audience is implied by the existing screens but not yet confirmed as a separate role: the people who connect and maintain the environments (addresses, service logins, certificates).

## Product Purpose

Lemnos is an internal self-service console for compute. A developer signs in with the organisation's own identity, sees what is running, and powers their guests on and off, without needing an account on, or knowledge of, the hypervisor behind it.

## Positioning

A multi-provider control plane: one console over several infrastructure backends. Proxmox VE is the first provider, not the product. The same developer-facing experience is meant to hold whichever backend a workload runs on.

## Operating Context

- Deployed inside an organisation, on its own hardware.
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

Open decisions:

- Roles. Every signed-in user currently sees every environment, including its address and service login, and can remove it. Whether developers and environment administrators get different views is undecided.
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
