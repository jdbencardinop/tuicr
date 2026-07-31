# Install — offline-candidate build

This archive is an **offline-validated fork candidate**, not a tagged
release. Treat it as a hands-on validation build, not something to roll out
broadly.

## Install

```bash
tar xzf tuicr-offline-candidate-@@VERSION@@-@@SOURCE_SHA_SHORT@@-@@OS@@-@@ARCH@@.tar.gz
cd tuicr-offline-candidate-@@VERSION@@-@@SOURCE_SHA_SHORT@@-@@OS@@-@@ARCH@@
chmod +x tuicr
./tuicr --version   # expect: tuicr @@FORK_VERSION@@+@@SOURCE_SHA_SHORT@@ (see README.md re: binary identity)
```

Put `tuicr` somewhere on your `PATH` (e.g. `~/.local/bin/tuicr`) if you want
to run it as `tuicr` instead of `./tuicr`. There is no installer script; this
is a plain static-ish binary drop, matching upstream's own release layout.

macOS users: because this binary is not notarized/signed by this fork,
Gatekeeper may quarantine it on first run. If macOS refuses to run it:

```bash
xattr -d com.apple.quarantine ./tuicr
```

## Automatic update check and `tuicr update` (disabled, no action needed)

This build permanently disables the automatic startup crates.io check and
the `tuicr update` subcommand at the source level — see README.md's
"`tuicr update` is disabled in this build" section. This binary will never
contact crates.io, GitHub Releases, Homebrew, cargo, or mise on its own. No
config or flag is required to achieve this. `config.no-update-check.toml`
is still bundled below purely as a documented sample of the (now
no-op-for-this-purpose) `no_update_check` config key:

```bash
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/tuicr-offline-candidate"
cp config.no-update-check.toml "${XDG_CONFIG_HOME:-$HOME/.config}/tuicr-offline-candidate/config.toml"
```

(macOS also uses `${XDG_CONFIG_HOME:-~/.config}/tuicr-offline-candidate` for
config, per `MIGRATION.md` — config is XDG-uniform across both platforms
even though review-session data is not.)

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
rm -rf ~/Library/Application\ Support/tuicr-offline-candidate   # macOS review-session data
rm -rf ~/.local/share/tuicr-offline-candidate                    # Linux review-session data
rm -rf ~/.config/tuicr-offline-candidate                         # config/themes, if created
```

Uninstalling does not touch your Git repositories, any provider account, or
a real upstream `tuicr` install's own data/config directories — this fork
uses the fork-specific `tuicr-offline-candidate` directories described in
`MIGRATION.md`, never upstream's `tuicr` ones.
