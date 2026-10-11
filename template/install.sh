#!/usr/bin/env bash
# Builds the plugin and installs it where DAWs look for plugins (macOS user folders).
set -euo pipefail
cd "$(dirname "$0")"
cargo xtask bundle plugin --release
name=$(sed -n 's/^name = "\(.*\)"/\1/p' bundler.toml)
vst3="$HOME/Library/Audio/Plug-Ins/VST3"
clap="$HOME/Library/Audio/Plug-Ins/CLAP"
mkdir -p "$vst3" "$clap"
for dest in "$vst3/$name.vst3" "$clap/$name.clap"; do
  [ -e "$dest" ] && /usr/bin/trash "$dest"
done
cp -R "target/bundled/$name.vst3" "$vst3/"
cp -R "target/bundled/$name.clap" "$clap/"
echo "Installed $name to $vst3 and $clap. Rescan plugins in your DAW."
