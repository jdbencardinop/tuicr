# Install — offline-candidate build

This archive is an **offline-validated fork candidate**, not a tagged
release. Treat it as a hands-on validation build, not something to roll out
broadly.

## Install

```bash
tar xzf tuicr-offline-candidate-@@VERSION@@-@@SOURCE_SHA_SHORT@@-@@OS@@-@@ARCH@@.tar.gz
cd tuicr-offline-candidate-@@VERSION@@-@@SOURCE_SHA_SHORT@@-@@OS@@-@@ARCH@@
chmod +x tuicr
./tuicr --version   # expect: tuicr @@VERSION@@ (see README.md re: binary identity)
```

Put `tuicr` somewhere on your `PATH` (e.g. `~/.local/bin/tuicr`) if you want
to run it as `tuicr` instead of `./tuicr`. There is no installer script; this
is a plain static-ish binary drop, matching upstream's own release layout.

macOS users: because this binary is not notarized/signed by this fork,
Gatekeeper may quarantine it on first run. If macOS refuses to run it:

```bash
xattr -d com.apple.quarantine ./tuicr
```

## Disable the automatic update check (recommended)

This binary's crate/repository identity is still unmodified upstream
(`agavra/tuicr`), so its startup version-check phones home to crates.io and
`tuicr update`/package-manager upgrades would silently replace it with real
upstream — see README.md's "Do not run `tuicr update`" section for the full
explanation. At minimum, suppress the automatic startup check:

```bash
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/tuicr"
cp config.no-update-check.toml "${XDG_CONFIG_HOME:-$HOME/.config}/tuicr/config.toml"
```

(macOS also uses `${XDG_CONFIG_HOME:-~/.config}/tuicr` for config, per
`MIGRATION.md` — config is XDG-uniform across both platforms even though
review-session data is not.) Or pass `--no-update-check` on every
invocation instead. Either way, never run `tuicr update` itself.

## First run

```bash
cd /path/to/a/git/repo
tuicr --working-tree
```

opens the interactive review TUI against your uncommitted changes. See the
upstream `README.md`/`docs/` (bundled in the fork source, not this archive)
for keybindings.

## Non-interactive CLI

```bash
tuicr review list --repo .
tuicr review comments --session <slug>
tuicr review thread list --session <slug>
tuicr review publish --session <slug> --repo . --dry-run --provider github
```

`review publish --dry-run` never contacts a remote provider; it only reports
how each thread/reply/resolution would map onto the given provider's
capability profile.

## Uninstall

```bash
rm /path/to/tuicr        # wherever you placed the binary
rm -rf ~/Library/Application\ Support/tuicr   # macOS review-session data
rm -rf ~/.local/share/tuicr                    # Linux review-session data
rm -rf ~/.config/tuicr                         # config/themes, if created
```

Uninstalling does not touch your Git repositories or any provider account —
tuicr only ever writes to its own data/config directories described in
`MIGRATION.md`.
