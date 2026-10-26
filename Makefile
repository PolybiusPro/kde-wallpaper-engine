# Builds the Linux binary natively and the two Windows binaries for
# x86_64-pc-windows-gnu (needs that rust target and mingw-w64-gcc as the linker).
CARGO ?= cargo
CARGOFLAGS ?= --release --locked
WIN_TARGET = x86_64-pc-windows-gnu
WIN_DIR = target/$(WIN_TARGET)/release
PREFIX ?= /usr
COMMANDS = start stop control playback setup
VERSION = $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
DIST = kde-wallpaper-engine-$(VERSION)-x86_64

all:
	$(CARGO) build $(CARGOFLAGS) -p wpe-bridge
	$(CARGO) build $(CARGOFLAGS) --target $(WIN_TARGET) -p wpehost -p wtsapi32

check:
	$(CARGO) test --locked -p wpe-bridge
	$(CARGO) test --locked -p wpehost

# Needs cargo-llvm-cov and the llvm-tools-preview component. The windows-only
# glue in wpehost shows as uncovered on purpose; it cannot run on Linux.
coverage:
	$(CARGO) llvm-cov --locked -p wpe-bridge --summary-only
	$(CARGO) llvm-cov --locked -p wpehost --summary-only

# The release tarball: prebuilt binaries plus what install.sh needs, so the
# user does not need Rust. install.sh detects it by the missing Cargo.toml.
dist: all
	rm -rf dist/$(DIST)
	mkdir -p dist/$(DIST)/bin dist/$(DIST)/share
	install -m 755 target/release/wpe-bridge dist/$(DIST)/bin/
	install -m 644 $(WIN_DIR)/wpehost.exe $(WIN_DIR)/wtsapi32.dll dist/$(DIST)/bin/
	cp -r --no-preserve=mode kwin plasma dist/$(DIST)/
	install -m 644 share/wpe-bridge.desktop.in dist/$(DIST)/share/
	install -m 755 install.sh uninstall.sh dist/$(DIST)/
	install -m 644 README.md LICENSE dist/$(DIST)/
	tar -C dist --owner=0 --group=0 -czf dist/$(DIST).tar.gz $(DIST)
	cd dist && sha256sum $(DIST).tar.gz > $(DIST).tar.gz.sha256

clean:
	$(CARGO) clean
	rm -rf dist

# System-wide install for packaging (DESTDIR/PREFIX); install.sh is the per-user one.
# One binary serves every command; wpe-bridge-* are symlinks to it.
# wtsapi32.dll goes under share/wpe-bridge, and wpe-bridge-setup copies it into
# the user's Wallpaper Engine folder.
install:
	install -d $(DESTDIR)$(PREFIX)/bin $(DESTDIR)$(PREFIX)/share/wpe-bridge \
	    $(DESTDIR)$(PREFIX)/share/kwin/scripts $(DESTDIR)$(PREFIX)/share/plasma/wallpapers \
	    $(DESTDIR)/etc/xdg/autostart
	install -m 755 target/release/wpe-bridge $(DESTDIR)$(PREFIX)/bin/
	for f in $(COMMANDS); do \
	    ln -sf wpe-bridge $(DESTDIR)$(PREFIX)/bin/wpe-bridge-$$f; done
	install -m 644 $(WIN_DIR)/wpehost.exe $(WIN_DIR)/wtsapi32.dll \
	    $(DESTDIR)$(PREFIX)/share/wpe-bridge/
	cp -r --no-preserve=mode kwin/wpe-bridge $(DESTDIR)$(PREFIX)/share/kwin/scripts/
	cp -r --no-preserve=mode plasma/org.local.wpebridge $(DESTDIR)$(PREFIX)/share/plasma/wallpapers/
	sed "s#@BINDIR@#$(PREFIX)/bin#" share/wpe-bridge.desktop.in \
	    > $(DESTDIR)/etc/xdg/autostart/wpe-bridge.desktop
	install -Dm 644 LICENSE $(DESTDIR)$(PREFIX)/share/licenses/wpe-bridge/LICENSE

.PHONY: all check coverage dist clean install
