# Repackages the prebuilt release tarball, so no Rust or mingw is needed.
# Build with: rpmbuild -bb wpe-bridge.spec (tarball in SOURCES, or use spectool -g)
Name:           wpe-bridge
Version:        0.1.0
Release:        1
Summary:        Run Wallpaper Engine under Proton as the KDE Plasma wallpaper
License:        GPL-3.0-or-later
URL:            https://github.com/PolybiusPro/kde-wallpaper-engine
Source0:        %{url}/releases/download/v%{version}/kde-wallpaper-engine-%{version}-x86_64.tar.gz
ExclusiveArch:  x86_64
Requires:       plasma-workspace
Requires:       kwin
Requires:       kpipewire
Requires:       /usr/bin/kwriteconfig6
Requires:       systemd
Requires:       glibc >= 2.39
Recommends:     steam
# Prebuilt binaries: skip automatic dependency scanning, stripping and debuginfo.
AutoReqProv:    no
%global debug_package %{nil}
%global __os_install_post %{nil}

%description
Streams the real Windows Wallpaper Engine, running under Proton, into a
Plasma wallpaper plugin via PipeWire, with a KWin script placing the window.

%prep
%setup -q -n kde-wallpaper-engine-%{version}-x86_64

%install
install -d %{buildroot}%{_bindir} %{buildroot}%{_datadir}/wpe-bridge \
    %{buildroot}%{_datadir}/kwin/scripts %{buildroot}%{_datadir}/plasma/wallpapers \
    %{buildroot}%{_sysconfdir}/xdg/autostart
install -m 755 bin/wpe-bridge %{buildroot}%{_bindir}/
for f in start stop control playback setup; do
    ln -s wpe-bridge %{buildroot}%{_bindir}/wpe-bridge-$f
done
install -m 644 bin/wpehost.exe bin/wtsapi32.dll %{buildroot}%{_datadir}/wpe-bridge/
cp -r --no-preserve=mode kwin/wpe-bridge %{buildroot}%{_datadir}/kwin/scripts/
cp -r --no-preserve=mode plasma/org.local.wpebridge %{buildroot}%{_datadir}/plasma/wallpapers/
sed "s#@BINDIR@#%{_bindir}#" share/wpe-bridge.desktop.in \
    > %{buildroot}%{_sysconfdir}/xdg/autostart/wpe-bridge.desktop

%files
%license LICENSE
%{_bindir}/wpe-bridge*
%{_datadir}/wpe-bridge
%{_datadir}/kwin/scripts/wpe-bridge
%{_datadir}/plasma/wallpapers/org.local.wpebridge
%config(noreplace) %{_sysconfdir}/xdg/autostart/wpe-bridge.desktop
