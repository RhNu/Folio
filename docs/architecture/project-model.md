# Projects and dependencies

`folio.toml` defines a Folio project. Project discovery searches upward from the current directory for that exact filename, unless `--manifest-path` selects a manifest. A project has one source root and one output directory. Relative filesystem paths are resolved from the manifest directory.

## Manifest

The manifest uses the current release's fields and defaults, has no schema version, and rejects unknown fields. The supported language is Skyrim Papyrus, the build target is `skyrim-se`, and the output format is PEX.

The [Papyrus dialect reference](../papyrus/dialects.md) also documents Fallout and Starfield language requirements. Those reference sections do not add accepted manifest dialects or targets. A declaration dependency cannot enable another game's grammar or ABI.

```toml
[package]
name = "example-mod"
version = "0.2.0"

[paths]
source = "Source/Scripts"
output = "Scripts"

[languages.papyrus]
dialect = "skyrim"
extensions = ["psc"]
user-flags = ["ProjectFlag"]
fill-missing-arguments = false

[build]
target = "skyrim-se"
profile = "dev"
emit = ["pex"]
debug-info = true

[lint.rules]
"papyrus.prefer-truthy-none-check" = "warning"
```

The source and output paths default to `Source/Scripts` and `Scripts`. Both must be relative directories within the project, must not overlap, and must not point into `.folio` or contain symbolic links or other reparse points. The source directory must exist; the build creates the output directory. The build profile contributes to build identity but does not select an optimization level.

`user-flags` declares custom Papyrus flags. A string keeps the shorthand above; a table can set the PEX bit and allowed declaration sites:

```toml
user-flags = ["ProjectFlag", { name = "ApiTag", bit = 7, scopes = ["script", "function"] }]
```

Allowed scopes are `script`, `property`, `variable`, and `function` (including events). Omitted scopes mean all four. Bits 0 and 1 are reserved for built-in Hidden and Conditional; custom bits must be unique in 2–31. Omitted bits are allocated in case-insensitive name order from unused bits. Names must be ASCII identifiers and cannot redeclare keywords or built-in flags. Duplicate names, bits, scopes, empty scopes, and unsupported fields are errors. Metadata exposes flag definitions as objects. The normalized definitions participate in semantic inputs and build identity. Folio does not load `.flg` files.

`fill-missing-arguments` is disabled by default: omitting a required argument without a default is an error. Enabling it supplies type defaults at the call site and emits a warning without changing the function signature.

`build.debug-info` defaults to `true` and controls PEX function and line mappings. Both debug information and argument filling contribute to the build fingerprint. Lint severities are `off`, `info`, `warning`, and `error`; they do not change compilation validity for `check` or `build`.

## Dependency inputs

Each `[[dependencies]]` entry declares an independent API source, in manifest order.

| Kind | Path | Loading behavior |
| --- | --- | --- |
| `psc` | PSC directory | Recursively extract script APIs; decode UTF-8 by default or use `encoding = "windows1252"` |
| `pex` | PEX directory | Experimentally recover API facts present in the binary |
| `decl` | Declaration file | Detect JSON or binary `.fdecl` encoding from the contents |
| `repo` | Logical repository path | Find a declaration file in the local repository, then use the same declaration decoder and analysis path |

`name` is a unique source alias within the manifest; it need not match the declaration contents. Dependencies have no package version field, do not load another project's manifest, and do not resolve transitive dependencies. Filesystem paths for `psc`, `pex`, and `decl` may refer to local inputs outside the project or on another drive. Host paths and portable source identities are stored separately. The `encoding` option applies only to `psc`.

```toml
[[dependencies]]
name = "ck"
kind = "repo"
path = "ck/1.6.1170.0"

[[dependencies]]
name = "skse"
kind = "repo"
path = "skse/2.2.8"

[[dependencies]]
name = "other-mod"
kind = "psc"
path = "../OtherMod/Source/Scripts"

[[dependencies]]
name = "shared-api"
kind = "decl"
path = "../Declarations/shared-api.fdecl"
```

All four inputs become the same declaration model before script selection and semantic analysis. Dependency function bodies are excluded from compilation, and dependencies produce no deployable code in the current build. Root project sources retain their bodies; `build` generates PEX only for root project scripts.

PSC and PEX directories must be nonempty. Invalid declarations and duplicate script names within one directory input, compared without case sensitivity, are errors. PSC dependencies and declaration generation share directory scanning, decoding, and API extraction. Files and subdirectories inside directory inputs must be ordinary filesystem entries; links and special entry types are rejected.

## Script selection and runtime requirements

Script selection replaces whole scripts: later dependencies take precedence over earlier ones, and root project sources take precedence over all dependencies. Each dependency entry retains its position and provenance even when different aliases refer to the same carrier. Directory enumeration never sets priority.

Script names use ASCII case-insensitive comparison. A PSC filename must match its `ScriptName`. `folio tree` and `folio metadata` expose providers, selected sources, and external runtime requirements.

This provider order is Folio's project policy. The Skyrim reference compiler documents first-match selection in its import search path; that tool behavior does not override Folio's explicit manifest order. See [Reference compiler differences](../papyrus/runtime.md) for the distinction between source-language rules and tool workflows.

API visibility does not establish that a runtime implementation is installed. Folio neither downloads nor deploys dependencies and does not solve dependency versions.

## Local declaration repository

The user-level Folio home defaults to `.folio` in the user's home directory. Its `repo` subdirectory stores declarations. Set `FOLIO_HOME` to an absolute path to choose another location; the application fixes the location at the start of each operation.

```powershell
$env:FOLIO_HOME = 'D:\Folio'
```

A `repo` path uses `/` separators. It cannot contain an absolute path, drive letter, backslash, empty component, `.`, or `..`, and links must not allow it to escape `FOLIO_HOME/repo`. For example, `ck/1.6.1170.0` selects between:

```text
$FOLIO_HOME/repo/ck/1.6.1170.0.fdecl
$FOLIO_HOME/repo/ck/1.6.1170.0.json
```

If exactly one candidate exists, Folio uses it. If both exist, resolution reports ambiguity: specify the `.fdecl` or `.json` suffix in the manifest. An explicit suffix selects only that file. Candidate creation, deletion, and link target changes are input changes.

`folio declarations list` recursively lists decodable repository files with their provenance, profile, and script counts.

## Experimental PEX dependencies

The root project must explicitly enable direct PEX directory inputs:

```toml
[experimental]
pex-dependencies = true

[[dependencies]]
name = "compiled-mod"
kind = "pex"
path = "../CompiledMod/Scripts"
```

Folio reads Skyrim PEX 3.1 and 3.2 to extract scripts, inheritance, variables, properties, states, and callable members. PEX does not retain parameter defaults or distinguish whether a callable was originally a function or an event. The declaration model preserves these gaps as per-parameter `unknown` defaults and `unknown-callable` members.

Calls that supply every argument can be analyzed. Omitting an argument with an unknown default is an error even when `fill-missing-arguments` is enabled. A source declaration cannot override an inherited callable of unknown kind. These unknown facts survive JSON, binary declaration, and semantic input conversion. A declaration file's provenance label alone does not require the experimental option.

Corrupt PEX and formats unsupported by the codec are rejected. PEX and pregenerated declarations do not provide PSC definition locations on the consuming machine. Direct PSC dependencies can support navigation through the real source snapshot loaded for the operation.

## Declaration model

The declaration writer uses schema 2. Readers accept schema 1 JSON and its original binary tuple layout as well as schema 2. JSON contains a format identifier, schema version, language/ABI profile, generation provenance, and scripts. Provenance does not determine package identity or runtime capability.

```json
{
  "format": "folio-declarations",
  "schema": 2,
  "profile": "papyrus-skyrim",
  "origin": { "source": "self-authored-api" },
  "scripts": [
    {
      "name": "Example",
      "documentation": "An example API.",
      "members": [
        {
          "name": "Run",
          "documentation": "Run the requested number of iterations.",
          "kind": "function",
          "return_type": "Int",
          "native": true,
          "parameters": [
            { "name": "count", "ty": "Int", "default": { "kind": "literal", "value": "1" } }
          ]
        }
      ]
    }
  ]
}
```

`origin.source` is a portable descriptive label. The source generator also writes `input_digest`, covering sorted relative input paths and decoded source text. Script records can include inheritance, native status, flags, imports, states, members, and relative source locations. Those locations record generation history; consumers must not treat them as navigable paths on their machine. The semantic layer resolves type names.

Schema 2 adds optional `documentation` to scripts, states, and members. PSC extraction accepts `{ ... }` documentation immediately after script, property, and function headers; event documentation is a Folio extension. Source state, variable, inline, and statement documentation is rejected. Carrier state documentation remains representable. PEX extraction preserves existing script, property, and callable documentation strings. It does not invent documentation for states or variables, callable kinds, or parameter defaults.

Old carriers remain usable but cannot supply documentation they never retained. Regenerate from the original PSC inputs to add it. Encoding a schema 1 model writes schema 2; older Folio readers require an upgrade before consuming new carriers. Documentation and a schema-only upgrade are excluded from semantic API identity.

Member variants retain facts appropriate to `function`, `event`, `unknown-callable`, `property`, or `variable`. Omitting a function's `return_type` means it returns no value. Property `access` is `auto`, `auto-read-only`, or `manual` with `readable` and `writable` fields. Parameter defaults are `required`, `literal` with Papyrus literal text, or `unknown`. An omitted `default` means `required`. Parameter order is semantically significant.

PSC extraction shares pure declaration validation with source analysis: malformed headers, invalid property/accessor forms, modifier sites, default ordering, and invalid literal initializers/defaults are errors. A nonliteral initializer is never silently omitted. Callable body analysis remains excluded. Carrier validation checks structural facts; semantic consumption checks literal value/type compatibility without inventing unknown PEX facts. Reopened source states currently merge into one API state; CK acceptance of that extension remains unverified.

The current declaration model's `native` flag identifies an implementation supplied by the runtime. It does not encode whether a call is latent, synchronized with a game frame, or permitted in a particular runtime context. Names and the `native` flag alone cannot establish these properties. The [engine reference](../papyrus/runtime.md) records them separately; newer dialect constructs such as custom events, structs, access modifiers, and guards also require their own supported semantic representation before their declarations can be consumed as such.

## Declaration encoding

JSON and binary `.fdecl` use the same model and validator. Decoding identifies the encoding by contents.

The binary payload uses MessagePack arrays with fixed field positions and numeric tags, compressed with raw DEFLATE. A 60-byte header precedes the complete compressed stream:

| Field | Size and encoding |
| --- | --- |
| Magic | 8 bytes: `FOLDECL\0` |
| Schema | Little-endian `u32` |
| Uncompressed length | Little-endian `u64` |
| Compressed length | Little-endian `u64` |
| Payload digest | 32-byte BLAKE3 of the uncompressed payload |

The decoder checks lengths, the digest, complete stream consumption, structure, and tags; it rejects unknown tags and trailing data. Both the carrier and decompressed data are limited to 128 MiB, individual strings to 1 MiB, containers to 100,000 entries, and nesting to 64 levels. The decoder also limits the total number of values.

Array ordering and tag definitions belong to `modules/formats/declarations`. Rust memory layout is not the file protocol.

Schema 2 appends documentation fields to the script, state, and member arrays. Schema 1 is decoded with its own exact tuple definitions; missing fields are not inferred by accepting truncated schema 2 arrays.

## Generating declarations

Generation takes an explicit source directory, provenance label, and destination. Encoding defaults to UTF-8 and output to binary. Select exactly one destination: `--repo` for a repository logical path or `--output` for a filesystem path. Generation refuses to overwrite an existing file and publishes a complete file after rechecking the source snapshot.

```powershell
folio declarations generate --source-root '<CK PSC directory>' --source ck/1.6.1170.0 --encoding windows1252 --repo ck/1.6.1170.0
folio declarations generate --source-root '<SKSE PSC directory>' --source skse/2.2.8 --repo skse/2.2.8
folio declarations generate --source-root '<your PSC directory>' --source my-api --format json --output my-api.json
```

Generate CK and SKSE APIs separately and select each explicitly in the project manifest. Users with the relevant tools and script sources can generate declarations in their own local repository. Source code and derived declarations remain subject to the source's use and distribution terms. Retain actual tool versions and input digests in generation provenance; the repository path expresses the version label chosen by the user.

Inputs form a consistent snapshot for each operation. Semantic build fingerprints and physical file snapshots have separate roles; see [Builds and artifacts](build-artifacts.md).
