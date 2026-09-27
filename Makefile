# paintrs — native + web
#   make          build native release  -> target/release/paintrs
#   make run      run native
#   make web      build web release     -> dist/
#   make serve    dev server with live reload at http://127.0.0.1:8080
#   make test     unit tests
#   make check    clippy for native and web targets
#   make licenses regenerate THIRD_PARTY_LICENSES.txt (needs: cargo install cargo-about --features cli)

.PHONY: all run web serve test check licenses clean

all:
	cargo build --release

run:
	cargo run --release

web:
	trunk build --release

serve:
	trunk serve --release

test:
	cargo test

check:
	cargo clippy
	cargo clippy --target wasm32-unknown-unknown

licenses:
	cargo about generate --fail about.hbs -o THIRD_PARTY_LICENSES.txt

clean:
	cargo clean
	rm -rf dist
