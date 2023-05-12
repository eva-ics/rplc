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
	cross build --target x86_64-unknown-linux-musl --release --features cli
	cross build --target aarch64-unknown-linux-musl --release --features cli
	cd target/x86_64-unknown-linux-musl/release && cp rplc ../../../_build/rplc-${VERSION}-x86_64
	cd target/aarch64-unknown-linux-musl/release && \
		aarch64-linux-gnu-strip rplc && \
		cp rplc ../../../_build/rplc-${VERSION}-aarch64
	cd _build && echo "" | gh release create v$(VERSION) -t "v$(VERSION)" \
		rplc-${VERSION}-x86_64 \
		rplc-${VERSION}-aarch64
