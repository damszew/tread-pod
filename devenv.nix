{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

let
  # For packages that need a newer version than devenv-nixpkgs' pin (git-bug >= 0.12).
  pkgs-unstable = import inputs.nixpkgs-unstable { inherit (pkgs.stdenv) system; };

  # The Xtensa (ESP32-S3) Rust toolchain is not in nixpkgs: `esp-setup` installs it with espup,
  # and only the `fw-*` scripts use it. The toolchain and esp-idf-sys's ESP-IDF checkout live in a
  # per-user cache shared by every checkout and git worktree. Both are keyed by version (the
  # toolchain by the `esp-<version>` channel in firmware/rust-toolchain.toml, ESP-IDF by
  # `ESP_IDF_VERSION` in firmware/.cargo/config.toml), so branches can pin different versions.
  espEnv = ''
    ESP_SHARED="''${XDG_CACHE_HOME:-$HOME/.cache}/tread-pod/esp"
    ESP_CHANNEL="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$DEVENV_ROOT/firmware/rust-toolchain.toml")"
    ESP_TOOLCHAIN_VERSION="''${ESP_CHANNEL#esp-}"
    export RUSTUP_HOME="$ESP_SHARED/rustup"
    export CARGO_HOME="$ESP_SHARED/cargo"
    export ESP_IDF_TOOLS_INSTALL_DIR="custom:$ESP_SHARED/embuild"
    export PATH="${pkgs.rustup}/bin:$CARGO_HOME/bin:$PATH"
    unset RUSTFLAGS RUSTDOCFLAGS RUSTC CARGO RUST_SRC_PATH
    # LIBCLANG_PATH etc. for bindgen, written by espup.
    ESP_EXPORT="$ESP_SHARED/export-$ESP_CHANNEL.sh"
    if [ -f "$ESP_EXPORT" ]; then . "$ESP_EXPORT"; fi
    # Build scripts are linked by the Nix toolchain, so their loader doesn't search /usr/lib;
    # espup's prebuilt libclang needs libstdc++ from somewhere.
    export LD_LIBRARY_PATH="${
      lib.makeLibraryPath [ pkgs.stdenv.cc.cc.lib ]
    }''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  '';
in
{
  languages.rust = {
    enable = true;
    toolchainFile = ./rust-toolchain.toml;
  };

  packages = with pkgs; [
    # web UI
    trunk
    wasm-bindgen-cli # must match the `wasm-bindgen` pin in crates/web/Cargo.toml
    binaryen

    # firmware
    espup
    espflash
    ldproxy
    python3
    cmake
    ninja

    # host spike tools (bluer)
    pkg-config
    dbus
    bluez

    # misc
    convco
    taplo
    pkgs-unstable.git-bug # task tracking
  ];

  scripts = {
    check = {
      description = "fmt, clippy, tests, and a no_std build of engine";
      exec = ''
        set -e
        cargo fmt --all --check
        cargo clippy --workspace --all-targets -- -D warnings
        cargo test --workspace
        cargo build -p engine --target thumbv7em-none-eabihf
      '';
    };

    web-build = {
      description = "Build the Leptos web UI into crates/web/dist";
      exec = ''
        cd "$DEVENV_ROOT/crates/web" && trunk build --release --offline "$@"
      '';
    };

    sniff = {
      description = "Capture a treadmill's FTMS traffic into crates/engine/tests/fixtures/ftms (--help for options)";
      exec = ''
        cd "$DEVENV_ROOT" && cargo run -q --release -p ftms-sniff -- "$@"
      '';
    };

    web-dev = {
      description = "Run the api dev server and trunk serve (proxying /api)";
      exec = "devenv up";
    };

    esp-setup = {
      description = "Install the pinned Xtensa Rust toolchain (espup) into the shared ESP cache";
      exec = ''
        set -e
        ${espEnv}
        mkdir -p "$RUSTUP_HOME" "$CARGO_HOME"
        if rustup toolchain list | cut -d' ' -f1 | grep -qx "$ESP_CHANNEL"; then
          echo "$ESP_CHANNEL already installed in $ESP_SHARED"
        else
          espup install --targets esp32s3 --std \
            --toolchain-version "$ESP_TOOLCHAIN_VERSION" --name "$ESP_CHANNEL" \
            --export-file "$ESP_EXPORT"
        fi
      '';
    };

    fw-build = {
      description = "Build the ESP32-S3 firmware";
      exec = ''
        set -e
        ${espEnv}
        cd "$DEVENV_ROOT/firmware" && cargo build --release "$@"
      '';
    };

    fw-flash = {
      description = "Flash the firmware and open a serial monitor";
      exec = ''
        set -e
        ${espEnv}
        cd "$DEVENV_ROOT/firmware" && cargo run --release "$@"
      '';
    };

    fw-image = {
      description = "Write a merged flash image to firmware/target/tread-pod.bin";
      exec = ''
        set -e
        ${espEnv}
        cd "$DEVENV_ROOT/firmware"
        cargo build --release
        espflash save-image --chip esp32s3 --merge \
          target/xtensa-esp32s3-espidf/release/firmware target/tread-pod.bin
      '';
    };
  };

  # `devenv test` would otherwise start these dev servers around `enterTest`.
  processes = lib.mkIf (!config.devenv.isTesting) {
    api-dev.exec = "cargo run -p api --example dev_server --features dev-server";
    trunk.exec = "cd crates/web && trunk serve --offline";
  };

  git-hooks.hooks = {
    rustfmt.enable = true;
    clippy = {
      enable = true;
      settings.denyWarnings = true;
    };
    nixfmt.enable = true;
    taplo.enable = true;
    convco.enable = true;
  };

  enterShell = ''
    echo "tread-pod dev shell — scripts: check, sniff, web-build, web-dev, esp-setup, fw-build, fw-flash, fw-image"
  '';

  enterTest = ''
    check
  '';
}
