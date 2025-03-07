use crate::{
    portalnet::types::metric::{Metric, XorMetric},
    utils::bytes,
};

use discv5::enr::NodeId;
use ethereum_types::U256;

#[cfg(test)]
use crate::portalnet::Enr;
#[cfg(test)]
use discv5::enr::{CombinedKey, EnrBuilder};
#[cfg(test)]
use rand::Rng;
#[cfg(test)]
use std::net::Ipv4Addr;

/// Generate random NodeId based on bucket index target and a local node id.
/// First we generate a random distance metric with leading zeroes based on the target bucket.
/// Then we XOR the result distance with the local NodeId to get the random target NodeId
// TODO: We should be able to make this generic over a `Metric`.
pub fn generate_random_node_id(target_bucket_idx: u8, local_node_id: NodeId) -> NodeId {
    let distance_leading_zeroes = 255 - target_bucket_idx;
    let random_distance = bytes::random_32byte_array(distance_leading_zeroes);

    let raw_node_id = XorMetric::distance(&local_node_id.raw(), &random_distance);
    let raw_node_id_be = u256_to_be_bytes(raw_node_id);

    NodeId::new(&raw_node_id_be)
}

/// Returns the big-endian byte representation of a given `U256`.
fn u256_to_be_bytes(value: U256) -> [u8; 32] {
    let mut bytes: [u8; 32] = [0; 32];
    value.to_big_endian(&mut bytes);
    bytes
}

#[cfg(test)]
pub fn generate_random_remote_enr() -> (CombinedKey, Enr) {
    let key = CombinedKey::generate_secp256k1();

    let mut rng = rand::thread_rng();
    let ip = Ipv4Addr::from(rng.gen::<u32>());

    let enr = EnrBuilder::new("v4")
        .ip(ip.into())
        .udp(8000)
        .build(&key)
        .unwrap();

    (key, enr)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_generate_random_node_id_1() {
        let target_bucket_idx: u8 = 5;
        let local_node_id = NodeId::random();
        let random_node_id = generate_random_node_id(target_bucket_idx, local_node_id);
        let distance = XorMetric::distance(&random_node_id.raw(), &local_node_id.raw());
        let distance = u256_to_be_bytes(distance);

        assert_eq!(distance[0..31], vec![0; 31]);
        assert!(distance[31] < 64 && distance[31] >= 32)
    }

    #[test]
    fn test_generate_random_node_id_2() {
        let target_bucket_idx: u8 = 0;
        let local_node_id = NodeId::random();
        let random_node_id = generate_random_node_id(target_bucket_idx, local_node_id);
        let distance = XorMetric::distance(&random_node_id.raw(), &local_node_id.raw());
        let distance = u256_to_be_bytes(distance);

        assert_eq!(distance[0..31], vec![0; 31]);
        assert_eq!(distance[31], 1);
    }

    #[test]
    fn test_generate_random_node_id_3() {
        let target_bucket_idx: u8 = 255;
        let local_node_id = NodeId::random();
        let random_node_id = generate_random_node_id(target_bucket_idx, local_node_id);
        let distance = XorMetric::distance(&random_node_id.raw(), &local_node_id.raw());
        let distance = u256_to_be_bytes(distance);

        assert!(distance[0] > 127);
    }
}
