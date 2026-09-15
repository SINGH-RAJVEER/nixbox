# Package search

## Why NixBox keeps a local catalog

Running `nix search` for every edit makes interactive latency depend on evaluation state, network fetches, Nix caches, storage speed, and nixpkgs size. NixBox evaluates the target's locked nixpkgs revision once, stores the result, and answers later queries in process. This makes repeated search latency predictable without letting results drift away from the packages the target configuration will evaluate.

There is no webhook or background service. NixBox detects changes by reading `flake.lock` before searches and installs.

## Catalog source

At startup `PackageCatalog::load_or_build` reads `<configuration-root>/flake.lock` and follows this exact path through its JSON structure:

1. Read the root node identifier from `root`.
2. Read the root node's direct `inputs.nixpkgs` value.
3. Read that node's `locked` object.
4. Extract `type`, `owner`, `repo`, and `rev`.
5. Construct `github:<owner>/<repo>/<rev>` or `gitlab:<owner>/<repo>/<rev>`.

The catalog loader currently requires a direct root input represented by a node identifier string. It supports locked `github` and `gitlab` inputs. Indirect `follows` paths, tarballs, local paths, Git URLs without owner and repository fields, and lock files without a revision do not produce a catalog. NixBox reports the catalog error and uses live search instead.

## Building the catalog

When no current cache exists, NixBox runs the equivalent of:

```sh
nix --quiet search --json --extra-experimental-features "nix-command flakes" github:NixOS/nixpkgs/<locked-revision> '^
```

`^` asks Nix for the full searchable package set. NixBox allows at most 256 MiB of stdout for this catalog build and retains at most 64 KiB of stderr while still draining the complete error stream. If stdout crosses the limit, NixBox terminates and reaps the child process.

Nix returns keys such as `legacyPackages.x86_64-linux.ripgrep` and `legacyPackages.x86_64-linux.python312Packages.black`. NixBox removes only the `legacyPackages.<system>.` or `packages.<system>.` prefix. It preserves the remaining nested attribute because `python312Packages.black` is the expression needed in `pkgs.python312Packages.black`.

After parsing, NixBox sorts hits by normalized attribute, removes duplicate attributes, stores the format version and source revision with the data, serializes to a temporary file, and renames the temporary file to `package-catalog.json`. A partially written temporary file never becomes the active cache.

## Loading and invalidation

The cache is accepted only when all of these values match:

- Catalog format version.
- Locked flake reference.
- Locked revision.

NixBox checks the lock again before each scheduled local search and before installing a selected package. If the lock changed, it drops the in-memory catalog, starts a rebuild, and waits to search or install against the new revision. This polling design works for `nixpkgs-unstable` because the revision, not the moving branch name, identifies the catalog.

Removing `~/.cache/nixbox/package-catalog.json` forces a rebuild on the next start. There is no manual refresh key in the current TUI.

## Query execution

Editing the package query aborts the previous scheduled task and increments `search_epoch`. An empty query clears results. A non-empty query waits 180 ms, then runs local ranking in a blocking Tokio worker so scanning the catalog does not stall terminal input or drawing.

Completion events carry the epoch captured when the task started. The TUI applies results only if that epoch still matches, which protects against an old query completing after a newer query.

The local search keeps a binary heap containing at most 200 candidates while scanning every catalog document. It lowercases each query and the searchable fields but returns the original values for display and installation.

## Ranking

Lower tiers rank first. Within a tier, an earlier match ranks first, then a shorter package name, then lexical package-name order.

| Tier | Match |
| --- | --- |
| 0 | Exact package name or attribute. |
| 1 | Package name or attribute prefix. |
| 2 | Complete alphanumeric token in package name or attribute. |
| 3 | Substring in package name or attribute. |
| 4 | Complete alphanumeric token in description. |
| 5 | Substring in description. |
| 6 | No match. The item is omitted. |

The ranking helper strips a leading `^` and trailing `$` before comparison. Token boundaries are any non-alphanumeric character. Underscores and hyphens therefore delimit tokens for ranking even though they remain valid in Nix package attributes.

## Live fallback

If catalog preparation fails, searches call `nix search --json` with the current query and configured channel. Known channel aliases resolve as follows:

| Setting | Live flake reference |
| --- | --- |
| `nixpkgs` | `github:NixOS/nixpkgs/nixos-26.05` |
| `nixpkgs-26.05` | `github:NixOS/nixpkgs/nixos-26.05` |
| `nixpkgs-unstable` | `github:NixOS/nixpkgs/nixos-unstable` |
| Any other value | Passed directly to Nix. |

Live search limits stdout to 50 MiB, retains 64 KiB of stderr, ranks the returned JSON with the same tiers, and returns no more than 200 hits. It may still have the variable evaluation latency that the catalog is designed to avoid.

## Performance measurements

The implementation benchmark recorded these debug-build timings on one development machine with 112,847 packages:

| Operation | Observed time |
| --- | --- |
| Current live `nix search` baseline | 15.9 seconds to more than 45 seconds. |
| Initial catalog build and load with a warm Nix evaluation cache | 2.956 seconds. |
| Loading the existing 16 MiB catalog from disk | 425 ms. |
| Local search for `firefox` | 53 ms. |
| Local search for `ripgrep` | 75 ms. |

These figures describe one machine and cache state. Catalog build time still depends on Nix evaluation and fetching. The repeatable gain comes from removing Nix from the query path after the matching catalog is loaded.

## Failure behavior

- A missing or invalid `flake.lock`, a missing direct nixpkgs input, or an unsupported locked input type disables the catalog for that run and activates live search.
- A corrupt or outdated cache is ignored and rebuilt.
- A failed catalog build leaves the previous cache file untouched because installation uses an atomic rename.
- A failed live search clears displayed results and puts the Nix error in the status line.
- Search-task cancellation aborts the Rust task. Spawned Nix children use `kill_on_drop`, so dropping the child handle requests termination.
