//! The device identity, from `irori-identity` (M1's loader moved there in
//! M2), plus the iroh view of it: the iroh `SecretKey` is the device ed25519
//! seed, so the `EndpointId` is the certified `devicePub` (D2).

pub use irori_identity::*;

/// The iroh key of a device identity.
pub trait IrohKey {
    /// The iroh secret key: the device ed25519 seed (D2).
    fn secret_key(&self) -> iroh::SecretKey;
    /// The device public key, which is also the iroh EndpointId.
    fn endpoint_id(&self) -> iroh::EndpointId {
        self.secret_key().public()
    }
}

impl IrohKey for DeviceIdentity {
    fn secret_key(&self) -> iroh::SecretKey {
        iroh::SecretKey::from_bytes(self.expose_seed())
    }
}
