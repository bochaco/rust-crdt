/*
This module is an implementation of LSeq CRDT, which makes use
of some basics/ideas from TreeDoc and Logoot CRDTS.

LSeq paper: https://hal.archives-ouvertes.fr/hal-00921633/document
TreeDoc paper: https://hal.inria.fr/inria-00445975/document
Logoot paper: https://hal.inria.fr/inria-00432368/document/
*/

mod lseq;
mod nodes;

use crate::traits::Causal;
use crate::vclock::{Actor, VClock};
pub use lseq::{LSeq, LSeqStrategy, Op};
use std::fmt::Display;

impl<V: Ord + Clone + PartialEq + Display, A: Actor + Display> PartialEq for LSeq<V, A> {
    // TODO: we need to compare the whole tree not just first level of siblings
    fn eq(&self, _other: &Self) -> bool {
        /*for (_, (dot, _)) in &self.siblings {
            let num_found = other.siblings.iter().filter(|(_, (d, _))| d == dot).count();

            if num_found == 0 {
                return false;
            }
            // sanity check
            assert_eq!(num_found, 1);
        }
        for (_, (dot, _)) in &other.siblings {
            let num_found = self.siblings.iter().filter(|(_, (d, _))| d == dot).count();

            if num_found == 0 {
                return false;
            }
            // sanity check
            assert_eq!(num_found, 1);
        }*/
        true
    }
}

impl<V: Ord + Clone + Eq + Display, A: Actor + Display> Eq for LSeq<V, A> {}

impl<V: Ord + Clone + Clone + Display, A: Actor + Display> Causal<A> for LSeq<V, A> {
    fn forget(&mut self, clock: &VClock<A>) {
        self.forget_clock(clock);
    }
}
