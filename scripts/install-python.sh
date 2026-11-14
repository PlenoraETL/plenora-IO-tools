#!/usr/bin/env bash
# CPython nativo sulla libc del container; minor dalla fonte unica.
set -euo pipefail
directory="$(cd "$(dirname "$0")" && pwd)"
minor="$(tr -d '\r\n' < "$directory/.python-version")"
version="$minor.6"
archive="Python-$version.tar.xz"
curl --fail --location --proto '=https' --tlsv1.2 \
  "https://www.python.org/ftp/python/$version/$archive" -o "/tmp/$archive"
echo "143b1dddefaec3bd2e21e3b839b34a2b7fb9842272883c576420d605e9f30c63  /tmp/$archive" | sha256sum --check -
tar -xJf "/tmp/$archive" -C /tmp
cd "/tmp/Python-$version"
./configure --prefix=/opt/plenora-python --with-ensurepip=install
make -j2
make install
/opt/plenora-python/bin/python3 -c "import sys; assert '.'.join(map(str, sys.version_info[:2])) == '$minor'"
