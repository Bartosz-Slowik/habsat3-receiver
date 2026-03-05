build-debug:
  cargo build --target aarch64-unknown-linux-gnu
  cargo build -p lora-listener --target x86_64-unknown-linux-gnu

  # Because we compile using nix toolchain, the program interpreter is some shared library in Nix store.
  # Our target does not have Nix store, therefore we override the interpreter path
  patchelf --set-interpreter /lib/ld-linux-aarch64.so.1 ./target/aarch64-unknown-linux-gnu/debug/air
  patchelf --set-interpreter /lib/ld-linux-aarch64.so.1 ./target/aarch64-unknown-linux-gnu/debug/bno-calibrate
  patchelf --set-interpreter /lib/ld-linux-aarch64.so.1 ./target/aarch64-unknown-linux-gnu/debug/lora-listener

  scp \
    target/aarch64-unknown-linux-gnu/debug/air \
    target/aarch64-unknown-linux-gnu/debug/bno-calibrate \
    target/aarch64-unknown-linux-gnu/debug/lora-listener \
    habsat@habsat.lan:~

  ssh habsat@habsat.lan 'mkdir --parents ~/.config/systemd/user'
  scp misc/air.service \
    habsat@habsat.lan:~/.config/systemd/user/

build-release:
  cargo build --release --target aarch64-unknown-linux-gnu
  cargo build --release -p lora-listener --target x86_64-unknown-linux-gnu

  # Because we compile using nix toolchain, the program interpreter is some shared library in Nix store.
  # Our target does not have Nix store, therefore we override the interpreter path
  patchelf --set-interpreter /lib/ld-linux-aarch64.so.1 ./target/aarch64-unknown-linux-gnu/release/air
  patchelf --set-interpreter /lib/ld-linux-aarch64.so.1 ./target/aarch64-unknown-linux-gnu/release/bno-calibrate
  patchelf --set-interpreter /lib/ld-linux-aarch64.so.1 ./target/aarch64-unknown-linux-gnu/release/lora-listener

  scp \
    target/aarch64-unknown-linux-gnu/release/air \
    target/aarch64-unknown-linux-gnu/release/bno-calibrate \
    habsat@habsat.lan:~

  ssh habsat@habsat.lan 'mkdir --parents ~/.config/systemd/user'
  scp misc/air.service \
    habsat@habsat.lan:~/.config/systemd/user/


mock-db-setup:
  mkdir --parents target/
  diesel setup --config-file apps/air/diesel.toml --database-url target/mock.db
  diesel migration run --config-file apps/air/diesel.toml --database-url target/mock.db 

db-regen:
  diesel migration redo --config-file apps/air/diesel.toml --database-url target/mock.db 

test:
  cargo test --target x86_64-unknown-linux-gnu \
    -p bmp280 \
    -p bno-055 \
    -p tape \
    -p system-sensors \
    -p tel0157 \
    -p as7341 \
    -p lora
