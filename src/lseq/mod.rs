/*
This module is an implementation of LSeq CRDT, which makes use
of some basics/ideas from TreeDoc and Logoot CRDTS.

LSeq paper: https://hal.archives-ouvertes.fr/hal-00921633/document
TreeDoc paper: https://hal.inria.fr/inria-00445975/document
Logoot paper: https://hal.inria.fr/inria-00432368/document/

This implementation stores the Identifiers in a tree structure,
which is expected to provide better performance and/or reuce the use
os storage needed as no redundant information is kept for Identifiers.
*/

mod lseq;
mod nodes;

pub use lseq::{LSeq, LSeqStrategy, Op};
