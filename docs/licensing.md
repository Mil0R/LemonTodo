# LemonTodo Licensing

## Decision

Adopt a Bitwarden-style licensing model.

This means LemonTodo should not use a single license for every file. Instead:

- Core client code uses GPLv3.
- Core server code uses AGPLv3.
- Selected future commercial or enterprise modules may use a LemonTodo source-available commercial license.
- Trademarks, names, logos, and official service branding are not granted by the code licenses.

## Rationale

This model supports:

- Public source code.
- Community review and contribution.
- Free self-hosting.
- Forking and modification under copyleft terms.
- Official hosted service monetization.
- Future commercial modules without closing the core product.

## Bitwarden Reference

Bitwarden uses a mixed model:

- Bitwarden clients are GPLv3 by default, with selected files under the Bitwarden License.
- Bitwarden server is AGPLv3 by default, with selected files under the Bitwarden License.
- Bitwarden's source-available license is not OSI open source.
- Self-hosting is free, but some premium features require a license file.

LemonTodo should follow the structure, not copy Bitwarden trademarks or license text without legal review.

## Proposed Repository License Layout

When implementation starts, use explicit license files similar to:

```text
LICENSE.txt
LICENSE_GPL.txt
LICENSE_AGPL.txt
LICENSE_LEMONTODO.txt
TRADEMARK_GUIDELINES.md
```

Recommended top-level `LICENSE.txt` meaning:

```text
Source code in this repository is covered by one of these licenses:

1. GNU General Public License v3.0 for client-side core and client apps.
2. GNU Affero General Public License v3.0 for server-side core.
3. LemonTodo License for selected source-available commercial modules.

The default license should be declared per directory or file header.
```

If the repository is a monorepo, avoid ambiguity by placing license notices in each crate or package.

Example:

```text
crates/tui       GPLv3
crates/core      GPLv3 or dual GPLv3/AGPLv3 depending on linkage needs
crates/crypto    GPLv3 or dual GPLv3/AGPLv3 depending on linkage needs
crates/server    AGPLv3
commercial/      LemonTodo License
```

The exact shared-core license needs legal review because both client and server may link to it.

## Commercial Boundary

Allowed under the core copyleft licenses:

- Personal use.
- Self-hosting.
- Modification.
- Redistribution under the same license obligations.
- Commercial use where GPLv3/AGPLv3 obligations are satisfied.

Restricted by trademark and future commercial module licensing:

- Using LemonTodo trademarks for an unofficial service.
- Selling an official-looking hosted service as LemonTodo.
- Using source-available commercial modules in production without a commercial license.

## Important Distinction

GPLv3 and AGPLv3 allow commercial use. They are open source licenses.

If LemonTodo wants a strict "modifiable but non-commercial" rule for all code, that would not match Bitwarden's core licensing model and would not be OSI open source.

The current project decision is to follow Bitwarden's model:

- open source core
- copyleft obligations
- source-available commercial modules later
- official hosted service monetization

## Legal Review Needed

Before public release, obtain legal review for:

- final license file wording
- shared-core license compatibility
- contributor license agreement or developer certificate of origin
- trademark policy
- paid hosted service terms
- source-available commercial module license

