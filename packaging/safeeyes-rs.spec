Name:           safeeyes-rs
Version:        0.1.1
Release:        1%{?dist}
Summary:        Lightweight, native Wayland Rust rewrite of SafeEyes

License:        MIT
URL:            https://github.com/zfgf11/safeeyes-rs
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust >= 1.85
BuildRequires:  pkgconfig(gtk4) >= 4.12
BuildRequires:  pkgconfig(gtk4-layer-shell-0)

Requires:       gtk4
Requires:       gtk4-layer-shell

%description
A lightweight, ultra-low-resource Rust rewrite of SafeEyes for Linux
(KDE Plasma and GNOME on Wayland), featuring a native StatusNotifierItem
system tray, responsive fullscreen break overlay, and event-driven idle
and screen-lock integration.

%prep
%autosetup -n %{name}-%{version}

%build
cargo build --release --locked

%install
install -Dm755 target/release/%{name} %{buildroot}%{_bindir}/%{name}
install -Dm644 packaging/%{name}.desktop %{buildroot}%{_datadir}/applications/%{name}.desktop

%check
cargo test --release --locked

%files
%license LICENSE
%doc README.md
%{_bindir}/%{name}
%{_datadir}/applications/%{name}.desktop

%changelog
* Fri Nov 13 2026 zfgf11 <zfgf11@users.noreply.github.com> - 0.1.1-1
- Add 30m, 2h, 4h, and indefinite options to "Disable for..." tray menu
- Add README screenshots and packaging updates

* Fri Nov 13 2026 zfgf11 <zfgf11@users.noreply.github.com> - 0.1.0-1
- Initial MVP release of safeeyes-rs
