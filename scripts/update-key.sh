#!/bin/sh
# Makes the key that signs the SHA256SUMS of each Runo release, once. The private key
# goes to the repository's UPDATE_SIGNING_KEY secret (the Release workflow signs with it)
# and stays in ~/.runo-update-key.pem; its public half goes into
# native/runo-app/src/update.pub, built into Runo, whose updater refuses a download
# whose checksums aren't signed with it.
set -eu
key="$HOME/.runo-update-key.pem"
root="$(cd "$(dirname "$0")/.." && pwd)"
if [ -e "$key" ]; then
  echo "$key already exists; using it."
else
  (umask 077 && openssl genpkey -algorithm ed25519 -out "$key")
  echo "Made $key. Keep it safe and offline: it is the only way to publish updates Runo trusts."
fi
openssl pkey -in "$key" -pubout -outform DER | tail -c 32 | base64 > "$root/native/runo-app/src/update.pub"
echo "Wrote the public key to native/runo-app/src/update.pub."
if command -v gh >/dev/null 2>&1; then
  gh secret set UPDATE_SIGNING_KEY < "$key"
  echo "Set the UPDATE_SIGNING_KEY secret."
else
  echo "Install the GitHub CLI and run: gh secret set UPDATE_SIGNING_KEY < $key"
fi
echo "Commit update.pub; Runo builds from then on only accept signed releases."
