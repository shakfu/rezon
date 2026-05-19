// OS-native credential storage wrapper. Backed by the `keyring`
// crate in `crates/rezon-web/src/secrets.rs` (Keychain on macOS,
// Credential Manager on Windows, secret-service on Linux).
//
// Account naming convention: `<purpose>:<scope>`. For cloud API
// keys, `api_key:<provider_key>` — e.g. `api_key:other`,
// `api_key:openai`. Stable across rezon versions; renaming an
// account orphans whatever was saved under the old name.

import { invoke } from "@tauri-apps/api/core";

export async function keychainGet(account: string): Promise<string | null> {
  return invoke<string | null>("keychain_get", { account });
}

export async function keychainSet(
  account: string,
  value: string,
): Promise<void> {
  return invoke<void>("keychain_set", { account, value });
}

export async function keychainDelete(account: string): Promise<void> {
  return invoke<void>("keychain_delete", { account });
}

/// Convention helper: account name for a cloud-provider API key.
/// Use this rather than constructing `"api_key:"+key` inline so
/// renames stay centralized.
export function cloudApiKeyAccount(providerKey: string): string {
  return `api_key:${providerKey}`;
}
