#!/bin/sh
# Makes the key that signs the agent manifests Runo downloads, once. The private key
# goes to the repository's REGISTRY_SIGNING_KEY secret (the Agent registry workflow signs
# with it) and stays in ~/.runo-registry-key.pem; its public half goes into
# native/runo-app/src/agents.pub, built into Runo, which checks every download with it.
set -eu
key="$HOME/.runo-registry-key.pem"
root="$(cd "$(dirname "$0")/.." && pwd)"
if [ -e "$key" ]; then
  echo "$key already exists; using it."
else
  (umask 077 && openssl genpkey -algorithm ed25519 -out "$key")
  echo "Made $key. Keep it safe: it is the only way to publish manifests Runo trusts."
fi
openssl pkey -in "$key" -pubout -outform DER | tail -c 32 | base64 > "$root/native/runo-app/src/agents.pub"
echo "Wrote the public key to native/runo-app/src/agents.pub."
if command -v gh >/dev/null 2>&1; then
  gh secret set REGISTRY_SIGNING_KEY < "$key"
  echo "Set the REGISTRY_SIGNING_KEY secret."
else
  echo "Install the GitHub CLI and run: gh secret set REGISTRY_SIGNING_KEY < $key"
fi
echo "Commit agents.pub; the next Runo release checks downloads with it."
