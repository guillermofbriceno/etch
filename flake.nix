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
      lib = pkgs.lib;

      # pinned to 1.93 due to issues with matrix-rust-sdk
      # https://github.com/matrix-org/matrix-rust-sdk/issues/6254
      rustToolchain = pkgs.rust-bin.stable."1.93.0".default;
      rustPlatform = pkgs.makeRustPlatform { cargo = rustToolchain; rustc = rustToolchain; };
      rustTarget = pkgs.stdenv.hostPlatform.rust.rustcTarget;

      # Named by the hash in the fork's 3rdparty/rnnoise-src/model_version.
      rnnoise-model = pkgs.fetchzip {
        url = "https://media.xiph.org/rnnoise/models/rnnoise_data-5e7841199cf2947fc32f6eeaf6faffab5c2dd3da69e6a6e4830625a10285a09d.tar.gz";
        hash = "sha256-EVVKJIDvZZdZjCV3M9qmHYYkKYudDDWvoLr31y7mJV0=";
        stripRoot = false;
      };

      # The Mumble fork with the extended plugin API (tag etch-mumble-patch),
      # configured like the dev build in ../mumble/shell.nix.
      etch-mumble = pkgs.stdenv.mkDerivation {
        pname = "etch-mumble";
        version = "1.7.0";

        src = pkgs.fetchFromGitHub {
          owner = "guillermofbriceno";
          repo = "mumble";
          rev = "d9c54ade27a941c938495d7e7ef2a081a5dd1289";
          fetchSubmodules = true;
          hash = "sha256-4CxuNI6OHNCCievjg16wHy616TNLxfWu2coIGAW6ezc=";
        };

        nativeBuildInputs = with pkgs; [
          cmake
          pkg-config
          python3
          qt6.qttools
          qt6.wrapQtAppsHook
        ];

        buildInputs = with pkgs; [
          qt6.qtbase
          qt6.qtsvg
          qt6.qtwayland
          openssl
          libopus
          libsndfile
          poco
          # Mumble 1.7's generated protobuf code does not build against protobuf 34.
          protobuf_33
          speechd
          glib
          zlib
          boost
          alsa-lib
          libpulseaudio
          (avahi.override { withLibdnssdCompat = true; })
          libglvnd
          libx11
          libxext
          libxi
          libxfixes
          libxrender
          libxcomposite
          libxtst
          libxcb
          libxcb-util
          libxcb-cursor
          libxcb-image
          libxcb-keysyms
          libxcb-render-util
          libxcb-wm
          libxkbcommon
        ];

        cmakeFlags = [
          "-Dclient=ON"
          "-Dserver=OFF"
          # The overlay needs a 32-bit multilib toolchain.
          "-Doverlay=OFF"
          "-Dplugins=ON"
          "-Dalsa=ON"
          "-Dpulseaudio=ON"
          "-Dspeechd=ON"
          "-Dzeroconf=ON"
          "-Dpipewire=OFF"
          "-Djackaudio=OFF"
          "-Dtests=OFF"
          "-Dupdate=OFF"
          "-Dwarnings-as-errors=OFF"
          # Otherwise CMake downloads the model at configure time, which the sandbox blocks.
          "-DFETCHCONTENT_SOURCE_DIR_RNNOISE_MODEL=${rnnoise-model}"
        ];
      };

      etch = rustPlatform.buildRustPackage (finalAttrs: {
        pname = "etch";
        inherit (lib.importJSON ./client/etch-gui/src-tauri/tauri.conf.json) version;

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./client ./plugin ./shared ];
        };

        cargoHash = "sha256-qer22IkXVr6xhJHgjgMgwAgMajOcsjqvbcQRFnMdkos=";

        pnpmDeps = pkgs.fetchPnpmDeps {
          inherit (finalAttrs) pname version src;
          pnpm = pkgs.pnpm_10;
          sourceRoot = "source/client/etch-gui";
          fetcherVersion = 3;
          hash = "sha256-Q5wPjtNCiUow4QzOovFgxcxsC2JXYJ6HLpfy6P2CO2Y=";
        };
        pnpmRoot = "client/etch-gui";

        buildAndTestSubdir = "client/etch-gui/src-tauri";

        # Updater artifacts need the release signing key, and Nix owns updates here.
        tauriBuildFlags = [ "--config" (builtins.toJSON { bundle.createUpdaterArtifacts = false; }) ];

        nativeBuildInputs = with pkgs; [
          (cargo-tauri.hook.override { cargo = rustToolchain; })
          nodejs
          pnpm_10
          pnpmConfigHook
          pkg-config
          wrapGAppsHook3
          rustPlatform.bindgenHook
        ];

        buildInputs = with pkgs; [
          openssl
          alsa-lib
          webkitgtk_4_1
          gtk3
          libsoup_3
          glib-networking
          librsvg
          libayatana-appindicator
          gst_all_1.gstreamer
          gst_all_1.gst-plugins-base
          gst_all_1.gst-plugins-good
          gst_all_1.gst-plugins-bad
          gst_all_1.gst-libav
        ];

        preBuild = ''
          cargo build --offline -j $NIX_BUILD_CORES --profile release --target ${rustTarget} -p etch-bridge
          mkdir -p client/etch-gui/src-tauri/bundled
          cp target/${rustTarget}/release/libetch_bridge.so client/etch-gui/src-tauri/bundled/
        '';

        postInstall = ''
          mkdir -p $out/lib/etch-gui/bundled/mumble/bin
          ln -s ${etch-mumble}/bin/mumble $out/lib/etch-gui/bundled/mumble/bin/mumble
        '';

        preFixup = ''
          gappsWrapperArgs+=(
            # The tray icon dlopens libayatana-appindicator at runtime.
            --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ pkgs.libayatana-appindicator ]}
          )
        '';

        # The workspace tests run in CI; several need containers and a network.
        doCheck = false;

        meta = {
          description = "Chat client built on Matrix, with voice over Mumble";
          homepage = "https://github.com/guillermofbriceno/etch";
          license = lib.licenses.gpl3Only;
          mainProgram = "etch-gui";
          platforms = [ system ];
        };
      });

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

        cargo-llvm-cov # for test coverage
      ];
    in
    {
      packages.${system} = {
        inherit etch etch-mumble;
        default = etch;
      };

      devShells.${system}.default = pkgs.mkShell {
        buildInputs = nativeDeps ++ [
          (rustToolchain.override {
            extensions = [ "rust-src" "rust-analyzer" "llvm-tools-preview" ];
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
