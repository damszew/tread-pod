{
  pkgs,
  lib,
  config,
  ...
}:

let
  # The Xtensa (ESP32-S3) Rust toolchain is not in nixpkgs: `esp-setup` installs it with espup
  # into project-local rustup/cargo homes, and only the `fw-*` scripts use it.
  espEnv = ''
    export RUSTUP_HOME="$DEVENV_STATE/esp/rustup"
    export CARGO_HOME="$DEVENV_STATE/esp/cargo"
    export PATH="${pkgs.rustup}/bin:$CARGO_HOME/bin:$PATH"
    unset RUSTFLAGS RUSTDOCFLAGS RUSTC CARGO RUST_SRC_PATH
    # LIBCLANG_PATH etc. for bindgen, written by espup.
    if [ -f "$DEVENV_STATE/esp/export-esp.sh" ]; then . "$DEVENV_STATE/esp/export-esp.sh"; fi
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
      description = "Install the Xtensa Rust toolchain (espup) into .devenv/state/esp";
      exec = ''
        set -e
        ${espEnv}
        mkdir -p "$RUSTUP_HOME" "$CARGO_HOME"
        if rustup toolchain list | grep -q '^esp'; then
          echo "esp toolchain already installed"
        else
          espup install --targets esp32s3 --std --export-file "$DEVENV_STATE/esp/export-esp.sh"
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
