# Run-time libraries of the Gold Band desktop app (a Tauri 2 app) on NixOS, where
# WebKitGTK and GTK are not on the loader path. The launcher that build-linux.sh
# installs enters this shell and reads GB_LIBS (LD_LIBRARY_PATH) and GB_DATA
# (XDG_DATA_DIRS). Nothing is installed into the system.
{ pkgs ? import <nixpkgs> { } }:
let
  libs = with pkgs; [
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
in
pkgs.mkShell {
  GB_LIBS = pkgs.lib.makeLibraryPath libs;
  GB_DATA = pkgs.lib.makeSearchPath "share" [ pkgs.gsettings-desktop-schemas pkgs.gtk3 pkgs.shared-mime-info ];
}
