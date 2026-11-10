# Build environment for the Gold Band desktop app (a Tauri 2 app) on NixOS:
# the Rust toolchain, Node for the web UI, and the GTK/WebKit libraries Tauri
# links against. `build-linux.sh` runs the build inside this shell; nothing is
# installed into the system. Use it with
#   nix-shell scripts/gold-band/shell.nix -I nixpkgs=flake:nixpkgs
{ pkgs ? import <nixpkgs> { } }:
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    cargo
    rustc
    gcc
    pkg-config
    nodejs_22
    git
    curl
    wrapGAppsHook3
  ];
  buildInputs = with pkgs; [
    webkitgtk_4_1
    gtk3
    libsoup_3
    glib
    cairo
    pango
    gdk-pixbuf
    atk
    librsvg
    openssl
    libayatana-appindicator
    libsecret
    dbus
  ];
}
