# kakeibo

A TUI and CLI for hledger journals: list transactions, browse accounts, and chart income and spending. The journal file is the source of truth. Text the app did not edit is written back byte for byte.

## Build

```
nix develop
cargo build --release
```

## Usage

```
kakeibo [--file PATH] [--config PATH] <command>

kakeibo              # TUI
kakeibo add          # add a transaction (flags, or prompts for missing fields)
kakeibo bal [ACCT]   # balances (--tree, --end YYYY-MM)
kakeibo reg [ACCT]   # register with running total
kakeibo check        # validate, report balance assertion failures
```

The journal is resolved in this order: `--file`, `$LEDGER_FILE`, `journal` in the config file, `~/.hledger.journal`.

```
kakeibo add -p Lidl -a expenses:groceries -m 57.30 -g assets:checking -n "weekly shop"
```

## TUI

Views: `1` Dashboard, `2` Transactions, `3` Accounts, `4` Reports. Press `?` for all keys.

Edits are staged in memory. `:w` writes atomically: it writes a temp file in the same directory, fsyncs it, and renames it over the journal. It also keeps `<journal>.bak`. `:q` asks for confirmation when there are unsaved changes. `u` and `Ctrl-r` undo and redo. `[` and `]` move the period. In the Transactions view they also shift a month filter.

The form edits date, payee, note, and any number of postings (`Ctrl-a` adds one, `Ctrl-x` removes one). One amount may be left empty; it is inferred. Payee and account fields autocomplete from the journal. Choosing a known payee fills in the postings from its last transaction.

`A` adds an account. It writes an `account NAME` directive after the existing declarations, or before the first transaction if there are none. In the Accounts view the prompt starts with the selected account's branch. An account also exists as soon as a transaction uses it, so typing a new name in the form works too.

If the file changes on disk, a clean buffer reloads silently. A dirty buffer prompts you to keep your version, reload theirs, or view a diff.

## Config

`~/.config/kakeibo/config.toml`:

```toml
journal = "~/finance/main.journal"
commodity = "PLN"

[keys]
add = "n"
redo = "ctrl-r, R"
```

Key action names are `dashboard`, `transactions`, `accounts`, `reports`, `down`, `up`, `top`, `bottom`, `page_down`, `page_up`, `open`, `add`, `add_account`, `edit`, `delete`, `search`, `search_next`, `search_prev`, `filter`, `prev_period`, `next_period`, `undo`, `redo`, `command`, `help`, and `quit`.

## Supported syntax

Supported:

- Dates in `YYYY-MM-DD`, `/` or `.` form
- Status `*` and `!`, and `(code)`
- `;` comments
- Postings with an account and amount, optionally with a `@` or `@@` cost and a `= amount` assertion
- One elided amount per transaction
- `account`, `commodity`, `payee` and `tag` directives
- `comment` blocks

The following are rejected, with the line number in the error:

- `include`
- Periodic (`~`) and automated (`=`) transactions
- Price directives
- Virtual postings
- Secondary dates
- Balance assignments
- `==` and `=*` assertions
- Ambiguous comma decimals
- A single tab between account and amount (hledger reads it as part of the account name)

Only one commodity is allowed per journal. It comes from the config, or else from the first amount in the file. Any other commodity is an error.

## Tests

```
nix develop -c cargo test
```

- `tests/roundtrip.rs`: parse and write is byte-identical for every fixture, and an edit changes only its own span.
- `tests/oracle.rs`: balances and `check` results match `hledger` (in the dev shell; skipped when hledger is missing).
- `tests/ui.rs`: insta snapshots of the screens.
