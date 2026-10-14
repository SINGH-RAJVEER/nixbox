# Deterministic commit messages

NixBox's proposed commit message should explain every recorded configuration operation in the order it happened. Fixed templates and a durable ordered journal provide this without a model, network request, or inference from the final diff. See [Version control](VERSION_CONTROL.md) for the user workflow and current behavior.

## Implementation

`crates/nixbox-core/src/vcs/message.rs` owns the fixed templates and pure renderer shared by the GUI, TUI, and CLI. New journal entries carry a redacted operation description, short subject summary, target, and changed paths. Migrations and option batches expand into individual entries in supplied order. The renderer adds sequence numbers without sorting, grouping, deduplicating, or canceling reversals. Long subjects fall back to a target summary while complete identifiers remain in the body. Control characters in identifiers are escaped. Older journals keep their recorded descriptions.

`vcs.rs` captures the journal before a configuration write under the existing journal lock. Git records HEAD; JJ records the working change ID and parent commit IDs, which remain stable across snapshots. App commits clear the reviewed journal. External commits reconcile it only after forward ancestry and a clean scoped diff, with all recorded paths inside the configuration root. Partial commits and uncertain branches or rewritten history retain entries for review. Legacy journals acquire a boundary on their next successful write. This preserves pending history rather than guessing which actions a partial commit included.

## Research findings

### Conventional changelog writer

The writer separates transforming structured commits from rendering the final text. Its defaults group by type, sort by header, and filter reverted commits. Those defaults serve release notes but discard the chronology and reversed actions required here. Its options and context implementation make sorting, grouping, and reverted-commit filtering separate decisions. NixBox should adopt the separation between records and rendering while preserving the journal's original sequence. [Default options source](https://github.com/conventional-changelog/conventional-changelog/blob/master/packages/conventional-changelog-writer/src/options.ts), [option types](https://github.com/conventional-changelog/conventional-changelog/blob/master/packages/conventional-changelog-writer/src/types/options.ts), [context and filtering source](https://github.com/conventional-changelog/conventional-changelog/blob/master/packages/conventional-changelog-writer/src/context.ts).

### Django migration descriptions

Django gives each operation a `describe()` method. Built-in operations produce concrete descriptions such as creating a model, deleting a model, and renaming one model to another. Its migration plan displays individual operations in sequence and describes the direction when reversing a migration. This is a useful model for a Rust enum with exhaustive, operation-specific templates. NixBox should retain package identifiers, selected flake outputs, targets, and individual option paths in those descriptions. Django also has operation reduction methods; reducing actions would lose the history this feature needs. [Operation documentation](https://docs.djangoproject.com/en/5.2/ref/migration-operations/#writing-your-own), [built-in description source](https://github.com/django/django/blob/stable/5.2.x/django/db/migrations/operations/models.py), [migration plan source](https://github.com/django/django/blob/stable/5.2.x/django/core/management/commands/migrate.py).

### Commit boundaries in Git and Jujutsu

Git documents `git rev-parse --verify HEAD` as a way to obtain the current commit's object name. `--verify` fails when it cannot resolve an object, and `^{commit}` additionally verifies the object type. A boundary record can therefore distinguish the initial state from an existing commit. [Git revision parsing](https://git-scm.com/docs/git-rev-parse#Documentation/git-rev-parse.txt---verify).

Jujutsu automatically snapshots edits by replacing the working-copy revision. A journal boundary cannot use the commit ID of `@`, because normal edits change that ID. The revset `@-` denotes its parent commits. For merge working copies, the boundary must represent all parents consistently rather than assume a single parent. [Working-copy behavior](https://docs.jj-vcs.dev/latest/working-copy/), [parent revsets](https://docs.jj-vcs.dev/latest/revsets/#functions).

Changing a revision alone does not prove that an external commit included every pending NixBox operation. This is an implementation constraint inferred from the VCS behavior: partial commits, amendments, rebases, and switching branches can all move the revision independently of journal coverage. Reconciliation must preserve operations whose inclusion cannot be established; it must not silently erase the journal on any revision change.

## Recommended design

- Generate a useful subject from the operation count and affected targets, then a numbered list of all recorded operations.
- Preserve journal append order. Do not alphabetize, group by operation type, deduplicate, or cancel an install followed by uninstall. Preserve the input order within package migrations and option batches too.
- Describe successful configuration writes. Queued, canceled, or failed writes must not claim that configuration changed. A later failed rebuild does not undo an already completed configuration write.
- Give each operation variant a fixed template. Expand multi-option operations into ordered set or unset descriptions, and distinguish flake packages from modules without Rust debug formatting.
- Omit option values from both the journal and message. Describe the action and option path. Escape control characters in identifiers so one operation cannot inject extra list entries or terminal controls.
- Keep rendering pure. Identical stored records must produce identical text regardless of locale, current time, filesystem enumeration order, or network access.
- Preserve legacy journal entries as recorded descriptions. Do not attempt to recover omitted values or reconstruct original chronology inside an old description.
- Clear only the journal entries approved in the successful commit review. Failed commits retain the journal. External commit reconciliation needs its own verified boundary behavior.
- State that manual edits have no operation history when the journal is empty. A final diff does not reveal the sequence of actions that produced it.

Tests should prove the contract with repeated and reversed operations, mixed targets, multi-option batches, legacy records, redacted values, control characters, unchanged output on repeated renders, and the journal's behavior across successful and failed commits. Git and JJ integration tests should exercise actual repository boundaries rather than mock revision strings.
