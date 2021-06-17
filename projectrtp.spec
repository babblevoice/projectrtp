Name: projectrtp
Version: 0
Release: 1%{?dist}
Summary: RTP Server

License: MIT
URL: https://github.com/tinpotnick/projectrtp
Source0: projectrtp.tar.gz

BuildRequires: make gcc
Requires: boost spandsp libsrtp

# build with debug 33M build without 1M - leave out for now.
%define debug_package %{nil}

%description
RTP Server for bridging or developing IVR applications.

%setup -q -n src

%prep
%autosetup

%build
make

%install
mkdir -p %{buildroot}%{_bindir}/
mkdir -p %{buildroot}%{_datadir}/projectrtp/sounds
install -m 755 ../out/projectrtp %{buildroot}%{_bindir}/projectrtp
install -m 644 ../out/uksounds.wav %{buildroot}%{_datadir}/projectrtp/sounds/uksounds.wav

%files
%{_bindir}/projectrtp
%{_datadir}/projectrtp/sounds/uksounds.wav

%license LICENSE

%changelog
* Wed Jun 16 2021 nick <nick@babblevoice.com>
-
