{
  description = "Etch development stack";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # v0.5.5
    #continuwuity.url = "github:continuwuity/continuwuity/55ccfdb9733347f1985206e782d6fd89e46c15c3";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay, ... }:
    let
      system = "x86_64-linux";
      overlays = [ (import rust-overlay) ];
      pkgs = import nixpkgs { inherit system overlays; };

      continuwuity-bin = pkgs.stdenv.mkDerivation {
        pname = "continuwuity-bin";
        version = "latest";

        src = pkgs.fetchurl {
          # Point this to their actual static musl release URL
          url = "https://forgejo.ellis.link/continuwuation/continuwuity/releases/download/v0.5.5/conduwuit-linux-amd64";

          hash = "sha256-USyMq4myeGwnlU3/3cxZ4nGMJnwp1ycUYUrA7lt+bXA=";
        };

        nativeBuildInputs = [ pkgs.autoPatchelfHook ];
        
        buildInputs = with pkgs; [
          libsecret
          liburing
          stdenv.cc.cc.lib # Usually needed for libgcc_s in C++ FFI
          zstd             # Adding this preemptively because RocksDB will probably ask for it next
        ];

        dontUnpack = true;

        installPhase = ''
          mkdir -p $out/bin
          # Rename the binary to match what your environment expects
          cp $src $out/bin/conduwuit
          chmod +x $out/bin/conduwuit
        '';
      };
      
      # Native dependencies for Tauri v2, Audio, and Network
      nativeDeps = with pkgs; [
        pkg-config
        gobject-introspection
        cargo-tauri
        nodejs_20
        pnpm
        libsecret
        
        # GTK & WebKit for Tauri GUI
        gtk3
        glib
        webkitgtk_4_1
        librsvg
        libsoup_3    # Common Tauri v2 network dep
        libayatana-appindicator # For system tray icons
        
        # Audio / Crypto for Mumble Core & Server
        openssl
        alsa-lib
        libopus
        protobuf
        sqlite

        continuwuity-bin

        gsettings-desktop-schemas

        # libclang for bindgen (Mumble plugin FFI)
        llvmPackages.libclang

        # Local CI testing (runs GitHub Actions in Docker)
        act

        # GStreamer for WebKitGTK media playback (<video>, <audio>)
        gst_all_1.gstreamer
        gst_all_1.gst-plugins-base  # appsink, playback, video/audio converters
        gst_all_1.gst-plugins-good  # matroska, isomp4, autodetect, pulseaudio
        gst_all_1.gst-plugins-bad   # openh264, webm/vp8/vp9
        gst_all_1.gst-libav         # ffmpeg-based decoders (h264, aac, etc.)

      ];
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        buildInputs = nativeDeps ++ [
          # pinned to 1.93 due to issues with matrix-rust-sdk
          # https://github.com/matrix-org/matrix-rust-sdk/issues/6254
          (pkgs.rust-bin.stable."1.93.0".default.override {
            extensions = [ "rust-src" "rust-analyzer" ];
          })
        ];

        shellHook = ''
          export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath nativeDeps}:$LD_LIBRARY_PATH
          # Fix small font for nixos wayland
          export XDG_DATA_DIRS=$GSETTINGS_SCHEMAS_PATH:$XDG_DATA_DIRS

          # This tells the 'mumble-protocol' build script exactly where protoc is
          export PROTOC="${pkgs.protobuf}/bin/protoc"

          # bindgen needs libclang
          export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"

          # Fixes a common blank-screen issue with WebKit on NixOS Wayland
          export GST_PLUGIN_PATH="${pkgs.gst_all_1.gst-plugins-base}/lib/gstreamer-1.0:${pkgs.gst_all_1.gst-plugins-good}/lib/gstreamer-1.0:${pkgs.gst_all_1.gst-plugins-bad}/lib/gstreamer-1.0:${pkgs.gst_all_1.gst-libav}/lib/gstreamer-1.0''${GST_PLUGIN_PATH:+:$GST_PLUGIN_PATH}"
          #export GST_PLUGIN_SYSTEM_PATH_1_0="$GST_PLUGIN_PATH"

          export WEBKIT_DISABLE_COMPOSITING_MODE=1

          # Point WebKit directly to the system fonts
          export FONTCONFIG_FILE=/etc/fonts/fonts.conf

          export CONDUWUIT_CONFIG="$PWD/conduwuit.toml"

          # This line prepends [etch-dev] to your prompt
          export PS1="\n\[\033[1;32m\][etch-dev] \[\033[0m\]\w\n\> "
          echo "Build environment ready. Workspace loaded."
        '';
      };
    };
}
