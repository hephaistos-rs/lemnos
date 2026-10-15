# Ideas

Things worth doing later. Nothing here is decided or built.

## Which machines belong to an organisation

Several organisations share one cluster, so Lemnos has to know which machines are whose. What is built today is the simplest version: an organisation is given an environment together with a tag, and it sees the machines in that environment that carry the tag. Tags are free text on the machine and anyone with rights on the Proxmox side can change them, so they are a filter, not a security boundary on Proxmox itself.

### Proxmox resource pools

A pool is Proxmox's own grouping of machines and storage. Unlike a tag, a machine is in at most one pool, and Proxmox permissions can be granted on a pool.

- One pool per organisation: `GET /pools/{poolid}` lists its members, so "the machines of organisation X" is one call instead of filtering the whole cluster.
- Lemnos could sign in with one API token per organisation that only has rights on that organisation's pool. Proxmox then enforces the boundary itself, even if Lemnos has a bug.
- A pool can also hold storage, which gives a natural place for "where this organisation's new disks go" once Lemnos creates machines.
- Cost: new calls in `lemnos-proxmox` (list pools, pool members), and a pool column beside the tag on the grant. A machine that is in no pool belongs to nobody.

### Other ways to tell machines apart

- **VM ID ranges** (organisation A owns 1000 to 1999). No Proxmox setup needed, but brittle and invisible to anyone looking at Proxmox.
- **A tag per owner as well** (`owner-alice`), for "my machines" inside an organisation.
- **Other providers.** Each has its own grouping (vSphere folders and resource pools, for example). The grant should stay "environment plus a provider-specific selector", so tags and pools are just two kinds of selector.

## Which organisation a person belongs to

Built today: an organisation can list sign-in providers in the config file; everyone who signs in through one of them joins it as a member. An admin can also add people by hand.

### Groups from the identity provider

Finer than "everyone from this provider", and the usual way to do it:

- **OIDC:** a `groups` claim in the ID token or from the userinfo endpoint. The claim name differs per provider, so it has to be configurable.
- **SAML:** a group attribute in the assertion (often `memberOf` or `groups`).
- **LDAP:** the `memberOf` attribute on the user, or a search for groups that list the user.
- Config would map a group to an organisation and a role, for example `"cn=platform,ou=groups,..." -> Platform, owner`.
- Open question: is the mapping applied on every sign-in (the identity provider is the truth, removing someone from the group removes them here) or only on the first (Lemnos is the truth afterwards)? Syncing on every sign-in is safer, but it fights with memberships an admin added by hand, unless those are marked as manual.
- Cost: `lemnos-auth` does not read groups today; its `Identity` type would need a list of them, filled by each of the three sign-in methods.

### Other

- **SCIM**, so the identity provider pushes users and groups to Lemnos instead of Lemnos learning them at sign-in. Also removes people who never sign in again.
- **Invitations by link**, for people outside the directory.

## Roles

Built today: one instance admin role, and owner, member and viewer inside an organisation.

- Custom roles built from permissions (start, stop, console, snapshot, create), if the three fixed ones turn out too coarse.
- Per-machine grants, for a contractor who needs exactly one VM.
- An audit log of who pressed which power button.

## Not started

- The machine's IP address in the list (needs the guest agent or the container's network config through `lemnos-proxmox`).
- The noVNC console (the connection exists in `lemnos-proxmox`; it needs a proxy route and a page).
- Filter, sort and search on Machines.
- Creating machines, quotas and requests.
- Legal pages and documentation for the public site.
