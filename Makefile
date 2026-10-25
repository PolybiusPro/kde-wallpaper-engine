# Builds the Linux binary natively and the two Windows binaries for
# x86_64-pc-windows-gnu (needs that rust target and mingw-w64-gcc as the linker).
CARGO ?= cargo
CARGOFLAGS ?= --release --locked
WIN_TARGET = x86_64-pc-windows-gnu
WIN_DIR = target/$(WIN_TARGET)/release
PREFIX ?= /usr
COMMANDS = start stop ui control playback setup

all:
	$(CARGO) build $(CARGOFLAGS) -p wpe-bridge
	$(CARGO) build $(CARGOFLAGS) --target $(WIN_TARGET) -p wpehost -p wtsapi32

check:
	$(CARGO) test --locked -p wpe-bridge
	$(CARGO) test --locked -p wpehost

clean:
	$(CARGO) clean

# System-wide install for packaging (DESTDIR/PREFIX); install.sh is the per-user one.
# One binary serves every command; wpe-bridge-* are symlinks to it.
# wtsapi32.dll goes under share/wpe-bridge, and wpe-bridge-setup copies it into
# the user's Wallpaper Engine folder.
install:
	install -d $(DESTDIR)$(PREFIX)/bin $(DESTDIR)$(PREFIX)/share/wpe-bridge \
	    $(DESTDIR)$(PREFIX)/share/kwin/scripts $(DESTDIR)$(PREFIX)/share/plasma/wallpapers \
	    $(DESTDIR)$(PREFIX)/share/applications $(DESTDIR)/etc/xdg/autostart
	install -m 755 target/release/wpe-bridge $(DESTDIR)$(PREFIX)/bin/
	for f in $(COMMANDS); do \
	    ln -sf wpe-bridge $(DESTDIR)$(PREFIX)/bin/wpe-bridge-$$f; done
	install -m 644 $(WIN_DIR)/wpehost.exe $(WIN_DIR)/wtsapi32.dll share/close-ui.js \
	    $(DESTDIR)$(PREFIX)/share/wpe-bridge/
	cp -r --no-preserve=mode kwin/wpe-bridge $(DESTDIR)$(PREFIX)/share/kwin/scripts/
	cp -r --no-preserve=mode plasma/org.local.wpebridge $(DESTDIR)$(PREFIX)/share/plasma/wallpapers/
	install -m 644 share/wpe-bridge-ui.desktop $(DESTDIR)$(PREFIX)/share/applications/
	sed "s#@BINDIR@#$(PREFIX)/bin#" share/wpe-bridge.desktop.in \
	    > $(DESTDIR)/etc/xdg/autostart/wpe-bridge.desktop
	install -Dm 644 LICENSE $(DESTDIR)$(PREFIX)/share/licenses/wpe-bridge/LICENSE

.PHONY: all check clean install
