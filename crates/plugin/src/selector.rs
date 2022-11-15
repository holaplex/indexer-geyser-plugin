use crate::interface::ReplicaAccountInfo;
use selector::prelude::*;
// use solana_program::message;

#[repr(transparent)]
pub struct AccountShim<'a>(pub &'a ReplicaAccountInfo<'a>);

impl<'a> AccountInfo for AccountShim<'a> {
    #[inline]
    fn owner(&self) -> &[u8] {
        self.0.owner
    }

    #[inline]
    fn pubkey(&self) -> &[u8] {
        self.0.pubkey
    }

    #[inline]
    fn data(&self) -> &[u8] {
        self.0.data
    }
}

// #[repr(transparent)]
// pub struct AccountKeyShim<'a>(pub message::AccountKeys<'a>);

// impl<'a> AccountKeys for AccountKeyShim<'a> {
//     #[inline]
//     fn get(&self, idx: usize) -> Option<&indexer_rabbitmq::geyser::Pubkey> {
//         self.0.get(idx)
//     }
// }
