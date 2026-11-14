// SPDX-License-Identifier: GPL-3.0-only
//! Live Chastity restore adapter for a Multipass program/service holding the
//! existing BOOT Shell route. No plaintext credential/account-file persistence.
//! This does not invent a server or bypass the provider's visible consent.
use crate::{address::Provider, flows::Env, service::Multipass, ApiError, Result};
use alloc::string::String;
use sex_app::{autofill::Client, Window};

pub struct Restore {
    client: Client,
    principal: String,
    provider: Provider,
    origin: String,
}
impl Restore {
    /// Call from the person's sign-in action while this consumer window has focus.
    /// `origin` is also the exact DAV base URL used for restored authentication.
    pub fn request(
        window: &Window,
        principal: &str,
        provider: Provider,
        origin: &str,
        now: u64,
    ) -> Result<Self> {
        // Validate before requesting a secret, using the same provider URL policy
        // as add_server_account. The final username is supplied only after consent.
        crate::address::server_account(provider, origin, "validation")?;
        let client = Client::request(window, origin, now).map_err(|_| {
            ApiError::new(
                "vault_request",
                "Chastity could not accept this sign-in request.",
            )
        })?;
        Ok(Self {
            client,
            principal: principal.into(),
            provider,
            origin: origin.into(),
        })
    }
    /// Poll at short intervals while active. Credentials exist only in the SDK's
    /// zeroizing buffer and the existing authentication call's borrowed closure.
    /// Multipass keeps the resulting session in Shibari, not its stamp file.
    pub fn poll(&mut self, env: &mut dyn Env, multipass: &mut Multipass) -> Option<Result<u64>> {
        let result = self.client.poll(env.now())?;
        Some(match result {
            Ok(credentials) => credentials.with(|user, password| {
                multipass.add_server_account(
                    env,
                    &self.principal,
                    self.provider,
                    &self.origin,
                    user,
                    password,
                )
            }),
            Err(_) => Err(ApiError::new(
                "vault_refused",
                "Chastity did not release credentials for this request.",
            )),
        })
    }
    pub fn cancel(&mut self) {
        self.client.cancel();
    }
}
