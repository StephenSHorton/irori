//! The iroh key of a device identity is the device ed25519 key (D2), so the
//! EndpointId equals the identity's `devicePub`.

use irori_net::identity::{DeviceIdentity, IrohKey as _};

#[test]
fn endpoint_id_is_the_device_pub() {
    // identity.md §7.1 SEED: seed 32x0x05.
    let id = DeviceIdentity::from_seed_for_tests([5u8; 32]);
    assert_eq!(
        id.endpoint_id().to_string(),
        "6e7a1cdd29b0b78fd13af4c5598feff4ef2a97166e3ca6f2e4fbfccd80505bf1"
    );
    assert_eq!(id.endpoint_id().to_string(), id.device_pub_hex());
    assert_eq!(*id.endpoint_id().as_bytes(), id.public_key());
}
