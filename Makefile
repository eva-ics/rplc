VERSION=$(shell grep ^version Cargo.toml|cut -d\" -f2)

all: @echo "Select target"

tag:
	git tag -a v${VERSION} -m v${VERSION}
	git push origin --tags

ver:
	sed -i 's/^version = ".*/version = "${VERSION}"/g' Cargo.toml
	sed -i 's/^const VERSION:.*/const VERSION: \&str = "${VERSION}";/g' src/main.rs

release: tag pkg

pkg:
	rm -rf _build
	mkdir -p _build
	cargo build --release --features cli
	cd target/release && cp rplc /opt/rplc/_build/rplc-${VERSION}-x86_64
	cross build --target aarch64-unknown-linux-gnu --release --features cli
	cd target/aarch64-unknown-linux-gnu/release && \
		aarch64-linux-gnu-strip rplc && \
		cp rplc /opt/rplc/_build/rplc-${VERSION}-aarch64
	cd _build && echo "" | gh release create v$(VERSION) -t "v$(VERSION)" \
		rplc-${VERSION}-x86_64 \
		rplc-${VERSION}-aarch64
