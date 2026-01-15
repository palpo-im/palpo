//! System utilities related to compute/processing

use std::path::PathBuf;
use std::sync::LazyLock;

type Id = usize;

type Mask = u128;
type Masks = [Mask; MASK_BITS];

const MASK_BITS: usize = 128;

/// The mask of logical cores available to the process (at startup).
static CORES_AVAILABLE: LazyLock<Mask> = LazyLock::new(|| into_mask(query_cores_available()));

/// Stores the mask of logical-cores with thread/HT/SMT association. Each group
/// here makes up a physical-core.
static SMT_TOPOLOGY: LazyLock<Masks> = LazyLock::new(init_smt_topology);

/// Stores the mask of logical-core associations on a node/socket. Bits are set
/// for all logical cores within all physical cores of the node.
static NODE_TOPOLOGY: LazyLock<Masks> = LazyLock::new(init_node_topology);

/// Get the number of threads which could execute in parallel based on hardware
/// constraints of this system.
#[inline]
#[must_use]
pub fn available_parallelism() -> usize {
    cores_available().count()
}

/// Gets the ID of the nth core available. This bijects our sequence of cores to
/// actual ID's which may have gaps for cores which are not available.
#[inline]
#[must_use]
pub fn nth_core_available(i: usize) -> Option<Id> {
    cores_available().nth(i)
}

/// Determine if core (by id) is available to the process.
#[inline]
#[must_use]
pub fn is_core_available(id: Id) -> bool {
    cores_available().any(|i| i == id)
}

/// Get the list of cores available. The values were recorded at program start.
#[inline]
pub fn cores_available() -> impl Iterator<Item = Id> {
    from_mask(*CORES_AVAILABLE)
}

// #[cfg(target_os = "linux")]
// #[inline]
// pub fn getcpu() -> Result<usize> {
//     use crate::{Error, utils::math};

//     // SAFETY: This is part of an interface with many low-level calls taking many
//     // raw params, but it's unclear why this specific call is unsafe. Nevertheless
//     // the value obtained here is semantically unsafe because it can change on the
//     // instruction boundary trailing its own acquisition and also any other time.
//     let ret: i32 = unsafe { nix::libc::sched_getcpu() };

//     #[cfg(target_os = "linux")]
//     // SAFETY: On modern linux systems with a vdso if we can optimize away the branch checking
//     // for error (see getcpu(2)) then this system call becomes a memory access.
//     unsafe {
//         std::hint::assert_unchecked(ret >= 0);
//     };

//     if ret == -1 {
//         return Err(Error::from_errno());
//     }

//     math::try_into(ret)
// }

// #[cfg(not(target_os = "linux"))]
// #[inline]
// pub fn getcpu() -> Result<usize, IoError> {
//     Err(IoError::new(ErrorKind::Unsupported, "not supported").into())
// }

fn query_cores_available() -> impl Iterator<Item = Id> {
    core_affinity::get_core_ids()
        .unwrap_or_default()
        .into_iter()
        .map(|core_id| core_id.id)
}

fn init_smt_topology() -> [Mask; MASK_BITS] {
    [Mask::default(); MASK_BITS]
}

fn init_node_topology() -> [Mask; MASK_BITS] {
    [Mask::default(); MASK_BITS]
}

fn into_mask<I>(ids: I) -> Mask
where
    I: Iterator<Item = Id>,
{
    ids.inspect(|&id| {
        debug_assert!(
            id < MASK_BITS,
            "Core ID must be < Mask::BITS at least for now"
        );
    })
    .fold(Mask::default(), |mask, id| mask | (1 << id))
}

fn from_mask(v: Mask) -> impl Iterator<Item = Id> {
    (0..MASK_BITS).filter(move |&i| (v & (1 << i)) != 0)
}

fn _sys_path(id: usize, suffix: &str) -> PathBuf {
    format!("/sys/devices/system/cpu/cpu{id}/{suffix}").into()
}
