use super::nodes::{Atom, AtomPayload, Identifier, MiniNodes, SiblingsNodes};
use crate::ctx::{AddCtx, ReadCtx, RmCtx};
use crate::traits::{Causal, CmRDT, CvRDT};
use crate::vclock::{Actor, VClock};
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::{
    cmp::{self, Ordering},
    fmt::{self, Display},
};

const DEFAULT_STRATEGY_BOUNDARY: u8 = 10;
const DEFAULT_STRATEGY: LSeqStrategy = LSeqStrategy::Alternate;
const DEFAULT_ROOT_BASE: u64 = 32; // This needs to be greater than boundary, and conveniently needs to be a power of 2

/// Strategy to be used when allocating a new identifier, which determines if new
/// id must be created under p or q
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LSeqStrategy {
    /// Deterministically chooses an stratey for each depth,
    /// a boundary+ is chosen if depth is even, and boundary- otherwise
    Alternate,
    /// Random stratey for each depth
    /// We may need to allow user to provide a seed if it needs to be deterministic
    Random,
    /// Boundary+ for all levels
    BoundaryPlus,
    /// Boundary- for all levels
    BoundaryMinus,
}

/// An LSeq, a variable-size identifiers class of sequence CRDT
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LSeq<V: Ord + Clone + Display, A: Actor + Display> {
    /// Boundary for choosing a new number when allocating an identifier
    boundary: u8,
    /// Arity of the root tree node. The arity is doubled at each depth
    root_arity: u64,
    /// The chosen allocation strategy
    strategy: LSeqStrategy,
    /// When inserting, we keep a cache of the strategy for each depth
    strategies: Vec<bool>, // true = boundary+, false = boundary-
    /// Depth 1 siblings nodes
    pub(crate) siblings: SiblingsNodes<V, A>,
    /// Clock with latest versions of all actors operating on this LSeq
    clock: VClock<A>,
}

impl<V: Ord + Clone + Display, A: Actor + Display> Default for LSeq<V, A> {
    fn default() -> Self {
        Self::new(
            DEFAULT_STRATEGY_BOUNDARY,
            DEFAULT_ROOT_BASE,
            DEFAULT_STRATEGY,
        )
    }
}

/// Defines the set of operations supported by LSeq
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op<V: Ord + Clone, A: Actor> {
    /// Insert a value in the sequence
    Insert {
        /// context of the operation
        clock: VClock<A>,
        /// the value to insert
        value: V,
        /// preceding value identifier (None == BEGIN)
        p: Option<Identifier>,
        /// succeeding value identifier (None == END, which can be used to append values)
        q: Option<Identifier>,
    },

    /// Delete a value from the sequence
    Delete {
        /// context of the operation
        clock: VClock<A>,
        /// the identifier of the value to delete
        id: Identifier,
    },
}

impl<V: Ord + Clone + Display, A: Actor + Display> Display for LSeq<V, A> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "|")?;
        for (i, (id, val)) in self.siblings.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}@{}", val, id)?;
        }
        write!(f, "|")
    }
}

impl<V: Ord + Clone + Display, A: Actor + Display> CmRDT for LSeq<V, A> {
    type Op = Op<V, A>;

    fn apply(&mut self, op: Self::Op) {
        match op {
            Op::Insert { clock, value, p, q } => {
                if clock.is_empty() {
                    return;
                }

                println!("\n\nINSERTING {} between {:?} and {:?}", value, p, q);

                // Allocate a new identifier between on p and q
                self.alloc_id(p, q, clock, value);
            }
            Op::Delete { id, clock } => {
                println!("\n\nDELETING {}", id);
                // Delete value from the atom which corresponds to the given identifier
                self.delete_id(id, clock);
            }
        }
    }
}

/// Implementation of the core LSeq functionality
impl<V: Ord + Clone + Display, A: Actor + Display> LSeq<V, A> {
    /// Construct a new empty LSeq with given boundary and root arity settings
    pub fn new(boundary: u8, root_arity: u64, strategy: LSeqStrategy) -> Self {
        Self {
            boundary,
            root_arity,
            strategy,
            strategies: vec![],
            siblings: SiblingsNodes::default(),
            clock: VClock::default(),
        }
    }

    /// Generate operation for inserting a value between identifiers p and q
    pub fn insert(
        &self,
        value: V,
        p: Option<Identifier>,
        q: Option<Identifier>,
        ctx: AddCtx<A>,
    ) -> Op<V, A> {
        Op::Insert {
            clock: ctx.clock,
            value,
            p,
            q,
        }
    }

    /// Generate operation to deleting a value given its identifier
    pub fn delete(&self, id: Identifier, ctx: RmCtx<A>) -> Op<V, A> {
        Op::Delete {
            clock: ctx.clock,
            id,
        }
    }

    /// Generates a read operation to obtain current state of the sequence
    pub fn read(&self) -> ReadCtx<Vec<(Identifier, V, VClock<A>)>, A>
    where
        V: Clone,
    {
        let sequence = self.flatten();
        ReadCtx {
            add_clock: self.clock.clone(),
            rm_clock: self.clock.clone(),
            val: sequence,
        }
    }

    /// Retrieve the current read context
    pub fn read_ctx(&self) -> ReadCtx<(), A> {
        ReadCtx {
            add_clock: self.clock.clone(),
            rm_clock: self.clock.clone(),
            val: (),
        }
    }

    // Private helpers functions

    /// Flatten tree into an ordered sequence of (Identifier, Value)
    fn flatten(&self) -> Vec<(Identifier, V, VClock<A>)> {
        let mut seq = vec![];
        self.flatten_tree(&self.siblings, Identifier::new(&[]), &mut seq);
        seq
    }

    /// Merge a clock into this LSeq global clock which keeps latest versions
    /// of all actors operating on this LSeq
    fn merge_clock(&mut self, clock: VClock<A>) {
        self.clock.merge(clock);
    }

    /// Returns the strategy corresponding to given depth and based on chosen by the user
    /// It also keeps a cache of the strategies for each depth as they are generated
    fn gen_strategy(&mut self, depth: usize) -> bool {
        if depth >= self.strategies.len() {
            // we need to add a new strategy to our cache
            let new_strategy = match self.strategy {
                LSeqStrategy::Alternate => {
                    if depth % 2 == 0 {
                        true
                    } else {
                        false
                    }
                }
                LSeqStrategy::Random => thread_rng().gen_bool(0.5),
                LSeqStrategy::BoundaryPlus => true,
                LSeqStrategy::BoundaryMinus => false,
            };
            self.strategies.push(new_strategy);
            new_strategy
        } else {
            self.strategies[depth]
        }
    }

    /// Returns the arity of the tree at a given depth
    fn arity_at(&self, depth: usize) -> u64 {
        self.root_arity << depth
    }

    /// Allocates a new identifier between given p and q identifiers
    pub(crate) fn alloc_id(
        &mut self,
        p: Option<Identifier>,
        q: Option<Identifier>,
        clock: VClock<A>,
        value: V,
    ) {
        let p = p.unwrap_or_else(|| Identifier::new(&[]));
        let q = q.unwrap_or_else(|| Identifier::new(&[]));

        // Let's get the interval between p and q, and also the depth at which
        // we should generate the new identifier
        let (new_id_depth, interval) = self.find_new_id_depth(&p, &q);
        println!(
            "INTERVAL FOUND: {} (new id depth: {})",
            interval, new_id_depth
        );

        // Let's make sure we allocate the new number within the preset boundary and interval obtained
        let step = cmp::min(interval, self.boundary as u64);

        // Depending on the strategy to apply, let's figure which is the new number
        let new_number = self.gen_new_number(new_id_depth, step, &p, &q);

        // Let's now attempt to insert the new identifier in the tree at new_id_depth
        let siblings_for_insert = self.find_siblings_in_tree(new_id_depth, &p);

        // We now need to insert the new number in the siblings we just looked up
        println!("New number {} for depth {}", new_number, new_id_depth);
        println!("INCOMING CLOCK: {}", clock);
        match siblings_for_insert.get(&new_number) {
            Some(Atom::Node { payload, children }) => {
                println!(
                    "Number {} already existing at depth {}",
                    new_number, new_id_depth
                );
                println!("CURRENT CLOCK: {}", payload.clock);
                println!("CLOCKS Comparison: {:?}", payload.clock.partial_cmp(&clock));

                match (payload.clock).partial_cmp(&clock) {
                    Some(Ordering::Less) => {
                        println!("Op's clock is newer, we don't allow this operation, cannot mutate a value TODO");
                        // TODO: perhaps find a new number to insert as it seems to be a brand new insert
                    }
                    None => {
                        println!("Concurrent operations!");
                        // Concurrent operations, we keep values a within mini nodes
                        // using the VClock<A> as the disambiguator

                        // Let's convert current Node into a MiniNodes to keep both values
                        // Insert both values in the mini_nodes
                        let mut mini_nodes = MiniNodes::default();
                        let new_atom_payload = AtomPayload {
                            clock: clock.clone(),
                            value: Some(value.clone()),
                        };

                        // We use the clock to order mini-nodes deterministically
                        mini_nodes.insert(new_atom_payload.clock.clone(), new_atom_payload);
                        mini_nodes.insert(payload.clock.clone(), payload.clone());

                        let new_atom = Atom::MajorNode {
                            payload: mini_nodes,
                            children: children.clone(),
                        };
                        siblings_for_insert.insert(new_number, new_atom);

                        // Merge clock into the LSeq's main clock
                        self.merge_clock(clock);
                    }
                    Some(Ordering::Equal) | Some(Ordering::Greater) => {
                        // Ignore it, we've already seen this operation
                    }
                }
            }
            Some(Atom::MajorNode { .. }) => {
                // TODO: depending on the clock, we may need to find a new number rather than
                // assume it's an insert between mini-nodes.

                // We don't support inserting between mini nodes
                println!("We don't support inserting between mini nodes");
            }
            None => {
                // It seems the slot picked is available, thus we'll use that one
                println!("It's a brand new identifier!");
                let children = SiblingsNodes::new();
                let payload = AtomPayload {
                    clock: clock.clone(),
                    value: Some(value.clone()),
                };
                let atom = Atom::Node { payload, children };
                siblings_for_insert.insert(new_number, atom);

                // Merge clock into the LSeq's main clock
                self.merge_clock(clock);

                println!(
                    "New number {} allocated at depth {}",
                    new_number, new_id_depth
                );
            }
        }
    }

    // Finds out what's the interval between p and q (regardless of their length/height),
    // and figure out which depth (either on p or q path) the new identifier should be generated at
    fn find_new_id_depth(&self, p: &Identifier, q: &Identifier) -> (usize, u64) {
        let mut interval: u64;
        let mut p_position = 0;
        let mut q_position = 0;
        let mut prev_q_position = 0;
        let mut new_id_depth = 0;

        loop {
            // Tree arity at current depth
            let arity = self.arity_at(new_id_depth);

            println!(
                "Checking interval at depth {} between {} and {:?}, arity {}...",
                new_id_depth, p, q, arity
            );
            if new_id_depth > 4 {
                panic!("STOP IT!");
            }
            let shift = new_id_depth + 2;

            // Calculate what would be the position in the sequence of p at current depth
            if new_id_depth < p.len() {
                let i = p.at(new_id_depth);
                p_position = (p_position << shift) + i;
            } else {
                // There is no number for p at this depth, thus we use
                // the equivalent of 0 in the range corresponding at this depth
                p_position = p_position << shift;
            }

            // Calculate what would be the position in the sequence of q at current depth
            if new_id_depth < q.len() {
                let i = q.at(new_id_depth);
                prev_q_position = i;
                q_position = (q_position << shift) + i;
            } else {
                // There is no number for q at this depth, thus we use the maximum
                // possible for this depth (it should be the same as arity of this depth - 1)
                q_position = if prev_q_position > 0 {
                    q_position << shift
                } else {
                    (q_position << shift) + arity
                };
                prev_q_position = 0;
            }

            println!("POS P: {}", p_position);
            println!("POS Q: {}", q_position);

            // What's the interval between p and q identifiers at current depth?
            interval = if p_position > q_position {
                // TODO: return error? the trait doesn't support that type of Result currently
                panic!("p cannot be greater than q");
            } else if q_position > p_position {
                q_position - p_position - 1
            } else {
                // p and q positions are equal
                0
            };

            // Did we reach a depth where there is room for a new id?
            if interval > 0 {
                break;
            } else {
                // ...nope...let's keep going
                new_id_depth = new_id_depth + 1;
            }
        }

        (new_id_depth, interval)
    }

    /// Get a new number to insert at given depth, and based on the depth's strategy
    fn gen_new_number(&mut self, depth: usize, step: u64, p: &Identifier, q: &Identifier) -> u64 {
        // Define if we should apply a boundary+ or boundary- stratey for the
        // new number, based on the depth where it's being added
        let strategy = self.gen_strategy(depth);

        // Depending on the strategy to apply, let's figure which is the reference number
        // we'll be adding to, or substracting from, to obtain the new number
        if strategy {
            // We then apply boundary+ strategy
            let reference_num = if depth < p.len() { p.at(depth) + 1 } else { 1 };

            // TODO: we may need a seed provided by the user to we get a deterministic result
            //let n = thread_rng().gen_range(reference_num, reference_num + step);
            let n = reference_num + (step / 2);
            println!("boundary+ (step {}): {}", step, n);
            n
        } else {
            // ...ok, then apply boundary- strategy
            let reference_num = if depth < q.len() {
                q.at(depth)
            } else {
                self.arity_at(depth) - 1 // == END at new id's depth
            };

            // TODO: we may need a seed provided by the user to we get a deterministic result
            //let n = thread_rng().gen_range(reference_num - step, reference_num);
            let n = reference_num - (step / 2);
            println!("boundary- (step {}): {}", step, n);
            n
        }
    }

    /// Find siblings in the tree at the level/depth where new number shall be inserted
    fn find_siblings_in_tree(&mut self, depth: usize, p: &Identifier) -> &mut SiblingsNodes<V, A> {
        // Let's now attempt to insert the new identifier in the tree at new_id_depth
        let mut cur_depth_nodes = &mut self.siblings;
        for cur_depth in 0..depth {
            // This is not yet the depth where to add the new number,
            // therefore we just check which child is the path of p/q at current's depth
            let cur_number = if cur_depth < p.len() {
                p.at(cur_depth)
            } else {
                0
            };

            // If there is no node for current number we create it so we can then step into it
            if !cur_depth_nodes.contains_key(&cur_number) {
                cur_depth_nodes.insert(
                    cur_number,
                    Atom::Node {
                        payload: AtomPayload {
                            clock: VClock::<A>::default(),
                            value: None,
                        },
                        children: SiblingsNodes::<V, A>::default(),
                    },
                );
            }

            // Now we can just step into the next depth of siblings to keep traversing the tree
            match cur_depth_nodes.get_mut(&cur_number) {
                Some(Atom::Node {
                    ref mut children, ..
                })
                | Some(Atom::MajorNode {
                    ref mut children, ..
                }) => {
                    cur_depth_nodes = children;
                }
                _ => {
                    // TODO: what if we didn't go through the complete identifier?
                    // do we have to create more than one new level? it shouldn't ever happen
                    panic!("Unexpected, it seems we need to create more than one new level?");
                }
            }
        }

        cur_depth_nodes
    }

    /// Forget given clock in each of the atoms' clock
    pub(crate) fn forget_clock(&mut self, clock: &VClock<A>) {
        // forget it from global clock maintained in the LSeq instance
        self.clock.forget(clock);

        // now forget it in each atom in the tree
        LSeq::forget_clock_in_tree(&mut self.siblings, clock);
    }

    /// Recursivelly forget the given clock in each of the atoms' clock
    fn forget_clock_in_tree(siblings: &mut SiblingsNodes<V, A>, c: &VClock<A>) {
        siblings.iter_mut().for_each(|s| match s {
            (
                _,
                Atom::Node {
                    payload: AtomPayload { ref mut clock, .. },
                    children: ref mut inner_siblings,
                },
            ) => {
                clock.forget(c);
                LSeq::forget_clock_in_tree(inner_siblings, c);
            }
            (
                _,
                Atom::MajorNode {
                    payload: ref mut mini_nodes,
                    children: ref mut inner_siblings,
                },
            ) => {
                mini_nodes.iter_mut().for_each(|(_, ref mut atom_value)| {
                    atom_value.clock.forget(c);
                });
                LSeq::forget_clock_in_tree(inner_siblings, c);
            }
        });
    }

    /// Find the atom in the tree following the path of the given identifier and delete its value
    pub(crate) fn delete_id(&mut self, mut id: Identifier, clock: VClock<A>) {
        let mut cur_depth_nodes = &mut self.siblings;
        let id_depth = id.len();
        for _ in 0..id_depth - 1 {
            let cur_number = id.remove(0);
            match cur_depth_nodes.get_mut(&cur_number) {
                Some(Atom::Node {
                    ref mut children, ..
                }) => {
                    cur_depth_nodes = children;
                }
                _ => {
                    // atom not found with given identifier
                    return;
                }
            }
        }

        if id.len() == 1 {
            match cur_depth_nodes.get(&id.at(0)) {
                Some(Atom::Node { ref children, .. }) => {
                    // found it as a node, we need to clear the value from it
                    let atom = Atom::Node {
                        payload: AtomPayload {
                            clock: clock.clone(),
                            value: None,
                        },
                        children: children.clone(),
                    };
                    cur_depth_nodes.insert(id.at(0), atom);
                    self.merge_clock(clock);
                }
                Some(Atom::MajorNode { .. }) => {
                    // found it as a mini node, we need to clear
                    // the value from the corresponding mini node
                    // TODO
                    //cur_depth_nodes.insert(id.at(0), new_atom);
                    self.merge_clock(clock);
                }
                None => { /* atom not found */ }
            }
        }
    }

    /// Recursivelly flattens the tree formed by the given siblings nodes
    /// The prefix is used for generating each Identifier in the sequence
    fn flatten_tree(
        &self,
        siblings: &SiblingsNodes<V, A>,
        prefix: Identifier,
        seq: &mut Vec<(Identifier, V, VClock<A>)>,
    ) {
        for (id, atom) in siblings {
            // We first push current node's number to the prefix
            let mut new_prefix = prefix.clone();
            new_prefix.push(*id);

            match atom {
                Atom::Node { payload, children } => {
                    // Add current atom's value to the sequence before processing children
                    if let Some(v) = &payload.value {
                        seq.push((new_prefix.clone(), v.clone(), payload.clock.clone()));
                    }
                    if !children.is_empty() {
                        self.flatten_tree(&children, new_prefix, seq);
                    }
                }
                Atom::MajorNode { payload, children } => {
                    // Add mini nodes to the sequence before processing children
                    self.flatten_atom_value(payload, new_prefix.clone(), seq);
                    if !children.is_empty() {
                        self.flatten_tree(&children, new_prefix, seq);
                    }
                }
            }
        }
    }

    /// Flattens the mini-nodes to form a sequence
    fn flatten_atom_value(
        &self,
        atom_value: &MiniNodes<V, A>,
        prefix: Identifier,
        seq: &mut Vec<(Identifier, V, VClock<A>)>,
    ) {
        // We return all mini-nodes here with the same Identifier,
        // this is ok for now as we don't support inserting between them.
        for (_, AtomPayload { clock, value }) in atom_value.iter() {
            if let Some(val) = value {
                seq.push((prefix.clone(), val.clone(), clock.clone()));
            };
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // helper to insert between two elements given their index in the sequence
    fn insert_between_i_j<V: Ord + Clone + Display, A: Actor + Display>(
        lseq: &mut LSeq<V, A>,
        i: Option<usize>,
        j: Option<usize>,
        value: V,
        actor: A,
    ) {
        let seq = lseq.read();
        let i_id = i.map(|index| seq.val[index].0.clone());
        let j_id = j.map(|index| seq.val[index].0.clone());
        let add_ctx = seq.derive_add_ctx(actor);
        let op = lseq.insert(value, i_id, j_id, add_ctx.clone());
        lseq.apply(op);
    }

    // helper to delete an element given its index in the sequence
    fn delete_index<V: Ord + Clone + Display, A: Actor + Display>(
        lseq: &mut LSeq<V, A>,
        index: usize,
    ) {
        let seq = lseq.read();
        let rm_ctx = seq.derive_rm_ctx();
        let id = &seq.val[index].0;
        lseq.apply(lseq.delete(id.clone(), rm_ctx.clone()));
    }

    #[test]
    fn test_append() {
        let mut lseq = LSeq::<char, u64>::new(1, 4, LSeqStrategy::BoundaryPlus);
        let actor = 100;

        // Append A to [] (between BEGIN and END)
        insert_between_i_j(&mut lseq, None, None, 'A', actor);

        // Append B to [A] (between A and END)
        insert_between_i_j(&mut lseq, Some(0), None, 'B', actor);

        // Append C to [A, B] (between B and END)
        insert_between_i_j(&mut lseq, Some(1), None, 'C', actor);

        // Append D to [A, B, C] (between C and END)
        insert_between_i_j(&mut lseq, Some(2), None, 'D', actor);

        // Test identifiers and values
        let seq_values = lseq.read().val;
        println!("FINAL SEQ: {:?}", seq_values);
        assert_eq!(seq_values.len(), 4);

        let (a_id, a_val, _) = &seq_values[0];
        let (b_id, b_val, _) = &seq_values[1];
        let (c_id, c_val, _) = &seq_values[2];
        let (d_id, d_val, _) = &seq_values[3];

        assert_eq!(*a_id, Identifier::new(&[1]));
        assert_eq!(*b_id, Identifier::new(&[2]));
        assert_eq!(*c_id, Identifier::new(&[3]));
        assert_eq!(*d_id, Identifier::new(&[3, 1]));

        assert_eq!(*a_val, 'A');
        assert_eq!(*b_val, 'B');
        assert_eq!(*c_val, 'C');
        assert_eq!(*d_val, 'D');
    }

    #[test]
    fn test_delete() {
        let mut lseq = LSeq::<char, u64>::new(1, 4, LSeqStrategy::BoundaryPlus);
        let actor = 100;

        // Append A to [] (between BEGIN and END)
        insert_between_i_j(&mut lseq, None, None, 'A', actor);

        // Append B to [A] (between A and END)
        insert_between_i_j(&mut lseq, Some(0), None, 'B', actor);

        // Append C to [A, B] (between B and END)
        insert_between_i_j(&mut lseq, Some(1), None, 'C', actor);

        // Delete B from [A, B, C]
        delete_index(&mut lseq, 1);

        // Test identifiers and values
        let seq_values = lseq.read().val;
        println!("FINAL SEQ: {:?}", seq_values);
        assert_eq!(seq_values.len(), 2);

        let (a_id, a_val, _) = &seq_values[0];
        let (c_id, c_val, _) = &seq_values[1];

        assert_eq!(*a_id, Identifier::new(&[1]));
        assert_eq!(*c_id, Identifier::new(&[3]));

        assert_eq!(*a_val, 'A');
        assert_eq!(*c_val, 'C');
    }

    #[test]
    fn test_insert_op_already_applied() {
        let mut lseq = LSeq::<char, u64>::new(10, 32, LSeqStrategy::BoundaryPlus);
        let actor1 = 100;
        let actor2 = 200;

        // actor1 inserts A to [] (between BEGIN and END)
        let add_ctx1 = lseq.read_ctx().derive_add_ctx(actor1);
        let op_actor1 = lseq.insert('A', None, None, add_ctx1.clone());
        lseq.apply(op_actor1.clone());

        // actor2 inserts B to [A] (between A and END)
        let seq = lseq.read();
        let add_ctx2 = seq.derive_add_ctx(actor2);
        let (a_id, _, _) = &seq.val[0];
        let op_actor2 = lseq.insert('B', Some(a_id.clone()), None, add_ctx2.clone());
        lseq.apply(op_actor2.clone());

        // lseq now sees both insert operations again as they were broadcasted by other sites again
        lseq.apply(op_actor1.clone());
        lseq.apply(op_actor2.clone());

        // Test that only two insert operations were persisted
        let seq_values = lseq.read().val;
        println!("FINAL SEQ: {:?}", seq_values);
        assert_eq!(seq_values.len(), 2);

        let (a_id, a_val, _) = &seq_values[0];
        let (b_id, b_val, _) = &seq_values[1];

        assert_eq!(*a_id, Identifier::new(&[6]));
        assert_eq!(*b_id, Identifier::new(&[12]));

        assert_eq!(*a_val, 'A');
        assert_eq!(*b_val, 'B');
    }

    #[test]
    fn test_insert_p_higher_depth_than_q() {
        let mut lseq = LSeq::<char, u64>::new(1, 4, LSeqStrategy::BoundaryPlus);
        let actor = 100;

        // insert A to [] (between BEGIN and END)
        insert_between_i_j(&mut lseq, None, None, 'A', actor);

        // insert B to [A] (between A and END)
        insert_between_i_j(&mut lseq, Some(0), None, 'B', actor);

        // insert C to [A, B] (between A and B)
        insert_between_i_j(&mut lseq, Some(0), Some(1), 'C', actor);

        // insert D to [A, C, B] (between C and B)
        insert_between_i_j(&mut lseq, Some(1), Some(2), 'D', actor);

        // Test identifiers and values are in correct order in the sequence
        let seq_values = lseq.read().val;
        assert_eq!(seq_values.len(), 4);
        let (a_id, a_val, _) = &seq_values[0];
        let (c_id, c_val, _) = &seq_values[1];
        let (d_id, d_val, _) = &seq_values[2];
        let (b_id, b_val, _) = &seq_values[3];
        println!("FINAL SEQ: {:?}", seq_values);

        assert_eq!(*a_id, Identifier::new(&[1]));
        assert_eq!(*b_id, Identifier::new(&[2]));
        assert_eq!(*a_val, 'A');
        assert_eq!(*b_val, 'B');

        assert_eq!(*c_id, Identifier::new(&[1, 1]));
        assert_eq!(*c_val, 'C');

        assert_eq!(*d_id, Identifier::new(&[1, 2]));
        assert_eq!(*d_val, 'D');
    }

    #[test]
    fn test_insert_q_higher_depth_than_p() {
        let mut lseq = LSeq::<char, u64>::new(1, 4, LSeqStrategy::BoundaryPlus);
        let actor = 100;

        // insert A to [] (between BEGIN and END)
        insert_between_i_j(&mut lseq, None, None, 'A', actor);

        // insert B to [A] (between A and END)
        insert_between_i_j(&mut lseq, Some(0), None, 'B', actor);

        // insert C to [A, B] (between B and END)
        insert_between_i_j(&mut lseq, Some(1), None, 'C', actor);

        // insert D to [A, B, C] (between C and END)
        insert_between_i_j(&mut lseq, Some(2), None, 'D', actor);

        // insert E to [A, B, C, D] (between B and D)
        insert_between_i_j(&mut lseq, Some(1), Some(3), 'E', actor);

        // Test identifiers and values are in correct order in the sequence
        let seq_values = lseq.read().val;
        assert_eq!(seq_values.len(), 5);
        let (a_id, a_val, _) = &seq_values[0];
        let (b_id, b_val, _) = &seq_values[1];
        let (e_id, e_val, _) = &seq_values[2];
        let (c_id, c_val, _) = &seq_values[3];
        let (d_id, d_val, _) = &seq_values[4];
        println!("FINAL SEQ: {:?}", seq_values);

        assert_eq!(*a_id, Identifier::new(&[1]));
        assert_eq!(*b_id, Identifier::new(&[2]));
        assert_eq!(*a_val, 'A');
        assert_eq!(*b_val, 'B');

        assert_eq!(*e_id, Identifier::new(&[2, 1]));
        assert_eq!(*e_val, 'E');

        assert_eq!(*c_id, Identifier::new(&[3]));
        assert_eq!(*c_val, 'C');

        assert_eq!(*d_id, Identifier::new(&[3, 1]));
        assert_eq!(*d_val, 'D');
    }

    #[test]
    fn test_insert_at_begining() {
        let mut lseq = LSeq::<char, u64>::new(10, 32, LSeqStrategy::Alternate);
        let actor = 100;

        // Insert A to [] (between BEGIN and END)
        insert_between_i_j(&mut lseq, None, None, 'A', actor);

        // Insert B to [A] (between BEGIN and A)
        insert_between_i_j(&mut lseq, None, Some(0), 'B', actor);

        // Insert C to [B, A] (between BEGIN and B)
        insert_between_i_j(&mut lseq, None, Some(0), 'C', actor);

        // Test identifiers and values are in correct order in the sequence
        let seq_values = lseq.read().val;
        println!("FINAL SEQ: {:?}", seq_values);
        assert_eq!(seq_values.len(), 3);
        let (c_id, c_val, _) = &seq_values[0];
        let (b_id, b_val, _) = &seq_values[1];
        let (a_id, a_val, _) = &seq_values[2];

        assert_eq!(*c_id, Identifier::new(&[2]));
        assert_eq!(*b_id, Identifier::new(&[3]));
        assert_eq!(*a_id, Identifier::new(&[6]));
        assert_eq!(*c_val, 'C');
        assert_eq!(*b_val, 'B');
        assert_eq!(*a_val, 'A');
    }

    #[test]
    fn test_insert_between_begin_and_first() {
        // in this test we try to insert between BEGIN and the very first Identifier,
        // in the scenario when there is no available slot between them and therefore a new level
        // is created with some 0 in the Identifier but ending with a non-0 number, e.g. [0,1]
        let mut lseq = LSeq::<char, u64>::new(1, 4, LSeqStrategy::BoundaryPlus);
        let actor = 100;

        // Insert A to [] (between BEGIN and END)
        insert_between_i_j(&mut lseq, None, None, 'A', actor);

        // Insert B to [A] (between BEGIN and A)
        insert_between_i_j(&mut lseq, None, Some(0), 'B', actor);

        // Insert C to [B, A] (between BEGIN and B)
        insert_between_i_j(&mut lseq, None, Some(0), 'C', actor);

        // Test identifiers and values are in correct order in the sequence
        let seq_values = lseq.read().val;
        println!("FINAL SEQ: {:?}", seq_values);
        assert_eq!(seq_values.len(), 3);
        let (c_id, c_val, _) = &seq_values[0];
        let (b_id, b_val, _) = &seq_values[1];
        let (a_id, a_val, _) = &seq_values[2];

        assert_eq!(*c_id, Identifier::new(&[0, 0, 1]));
        assert_eq!(*b_id, Identifier::new(&[0, 1]));
        assert_eq!(*a_id, Identifier::new(&[1]));
        assert_eq!(*c_val, 'C');
        assert_eq!(*b_val, 'B');
        assert_eq!(*a_val, 'A');
    }

    #[test]
    fn test_insert_between_last_and_end() {
        // in this test we try to insert between the very last Identifier possible
        // at current tree's height, and END (i.e. an append)
        let mut lseq = LSeq::<char, u64>::new(2, 4, LSeqStrategy::BoundaryPlus);
        let actor = 100;

        // Insert A to [] (between BEGIN and END)
        insert_between_i_j(&mut lseq, None, None, 'A', actor);

        // Insert B to [A] (between A and END)
        insert_between_i_j(&mut lseq, Some(0), None, 'B', actor);

        // Insert C to [A, B] (between B and END)
        insert_between_i_j(&mut lseq, Some(1), None, 'C', actor);

        // Insert D to [A, B, C] (between C and END)
        insert_between_i_j(&mut lseq, Some(2), None, 'D', actor);

        // Test identifiers and values are in correct order in the sequence
        let seq_values = lseq.read().val;
        println!("FINAL SEQ: {:?}", seq_values);
        assert_eq!(seq_values.len(), 4);
        let (a_id, a_val, _) = &seq_values[0];
        let (b_id, b_val, _) = &seq_values[1];
        let (c_id, c_val, _) = &seq_values[2];
        let (d_id, d_val, _) = &seq_values[3];

        assert_eq!(*a_id, Identifier::new(&[2]));
        assert_eq!(*b_id, Identifier::new(&[3]));
        assert_eq!(*c_id, Identifier::new(&[3, 2]));
        assert_eq!(*d_id, Identifier::new(&[3, 4]));
        assert_eq!(*a_val, 'A');
        assert_eq!(*b_val, 'B');
        assert_eq!(*c_val, 'C');
        assert_eq!(*d_val, 'D');
    }

    #[test]
    fn test_insert_concurrent() {
        let lseq = LSeq::<char, u64>::new(1, 4, LSeqStrategy::BoundaryPlus);
        let mut site1_seq = lseq.clone();
        let mut site2_seq = lseq.clone();
        let actor1 = 100;
        let actor2 = 200;

        let seq = lseq.read_ctx();
        let add_ctx1 = seq.derive_add_ctx(actor1);
        let add_ctx2 = seq.derive_add_ctx(actor2);

        // actor1 and actor2 insert concurrently A and B to [] (between BEGIN and END)
        let op_actor1 = lseq.insert('A', None, None, add_ctx1.clone());
        let op_actor2 = lseq.insert('B', None, None, add_ctx2.clone());

        // in site1 we see concurrent ops, first from actor1 then from actor2
        site1_seq.apply(op_actor1.clone());
        site1_seq.apply(op_actor2.clone());

        // in site2 we see concurrent ops in opposite order, first from actor2 then from actor1
        site2_seq.apply(op_actor2.clone());
        site2_seq.apply(op_actor1);

        // actor1 inserts a C to [A, B] (between B and END)
        let seq = site1_seq.read();
        let (b_id, _, _) = &seq.val[1];
        let add_ctx1 = seq.derive_add_ctx(actor1);
        let op_actor1 = site1_seq.insert('C', Some(b_id.clone()), None, add_ctx1.clone());
        site1_seq.apply(op_actor1.clone());
        site2_seq.apply(op_actor1);

        // lastly actor1 inserts a children of the MajorNode A/B (the node holding mini-nodes)
        // i.e. D to [A, B, C] (between A and C)
        let seq = site1_seq.read();
        let (a_id, _, _) = &seq.val[0];
        let (c_id, _, _) = &seq.val[2];
        let add_ctx1 = seq.derive_add_ctx(actor1);
        let op_actor1 = site1_seq.insert(
            'D',
            Some(a_id.clone()),
            Some(c_id.clone()),
            add_ctx1.clone(),
        );
        site1_seq.apply(op_actor1.clone());
        site2_seq.apply(op_actor1);

        // Test we read the exact same sequence from both sites
        let seq1_values = site1_seq.read().val;
        let seq2_values = site2_seq.read().val;
        println!("FINAL SEQ1: {:?}", seq1_values);
        println!("FINAL SEQ2: {:?}", seq2_values);
        assert_eq!(seq1_values.len(), 4);
        assert_eq!(seq2_values.len(), 4);
        assert_eq!(seq1_values[0], seq2_values[0]);
        assert_eq!(seq1_values[1], seq2_values[1]);
        assert_eq!(seq1_values[2], seq2_values[2]);
        assert_eq!(seq1_values[3], seq2_values[3]);

        // due to clock being the disambiguator, value A should be before B since actor1 (100) < actor2 (200)
        let (a_id, a_val, _) = &seq1_values[0];
        let (b_id, b_val, _) = &seq1_values[1];
        let (d_id, d_val, _) = &seq1_values[2];
        let (c_id, c_val, _) = &seq1_values[3];

        assert_eq!(*a_id, Identifier::new(&[1]));
        assert_eq!(*b_id, Identifier::new(&[1]));
        assert_eq!(*d_id, Identifier::new(&[1, 1]));
        assert_eq!(*c_id, Identifier::new(&[2]));
        assert_eq!(*a_val, 'A');
        assert_eq!(*b_val, 'B');
        assert_eq!(*d_val, 'D');
        assert_eq!(*c_val, 'C');
    }

    #[test]
    fn test_several_inserts() {
        let mut lseq = LSeq::<char, u64>::new(4, 4, LSeqStrategy::BoundaryPlus);
        let actor = 100;

        // Insert A to [] (between BEGIN and END)
        insert_between_i_j(&mut lseq, None, None, 'A', actor);

        // Insert B to [A] (between BEGIN and A)
        insert_between_i_j(&mut lseq, None, Some(0), 'B', actor);

        // Insert C to [B, A] (between B and A)
        insert_between_i_j(&mut lseq, Some(0), Some(1), 'C', actor);

        // Insert D to [B, C, A] (between C and A)
        insert_between_i_j(&mut lseq, Some(1), Some(2), 'D', actor);

        // Insert E to [B, C, D, A] (between B and C)
        insert_between_i_j(&mut lseq, Some(0), Some(1), 'E', actor);

        // Insert F to [B, E, C, D, A] (between D and A)
        insert_between_i_j(&mut lseq, Some(3), Some(4), 'F', actor);

        // Test identifiers and values are in correct order in the sequence [B, E, C, D, F, A]
        let seq_values = lseq.read().val;
        println!("FINAL SEQ: {:?}", seq_values);
        assert_eq!(seq_values.len(), 6);
        let (b_id, b_val, _) = &seq_values[0];
        let (e_id, e_val, _) = &seq_values[1];
        let (c_id, c_val, _) = &seq_values[2];
        let (d_id, d_val, _) = &seq_values[3];
        let (f_id, f_val, _) = &seq_values[4];
        let (a_id, a_val, _) = &seq_values[5];

        assert_eq!(*b_id, Identifier::new(&[1]));
        assert_eq!(*e_id, Identifier::new(&[1, 2]));
        assert_eq!(*c_id, Identifier::new(&[1, 3]));
        assert_eq!(*d_id, Identifier::new(&[1, 6]));
        assert_eq!(*f_id, Identifier::new(&[1, 7]));
        assert_eq!(*a_id, Identifier::new(&[2]));
        assert_eq!(*b_val, 'B');
        assert_eq!(*e_val, 'E');
        assert_eq!(*c_val, 'C');
        assert_eq!(*d_val, 'D');
        assert_eq!(*f_val, 'F');
        assert_eq!(*a_val, 'A');
    }

    #[test]
    #[should_panic]
    fn test_insert_p_greater_than_q() {
        let mut lseq = LSeq::<char, u64>::new(2, 2, LSeqStrategy::Alternate);
        let actor = 100;

        // Insert A to [] (between BEGIN and END)
        insert_between_i_j(&mut lseq, None, None, 'A', actor);

        // Insert B to [A] (between A and END)
        insert_between_i_j(&mut lseq, Some(0), None, 'B', actor);

        // Insert C to [A, B] (between B and A == wrong order)
        insert_between_i_j(&mut lseq, Some(1), Some(0), 'C', actor); // should panic
    }

    // TODO: test new insert finding used identifier (it now fails but it shouldn't)
    // TODO: test delete before insert, and insert between Identifiers which are still unknown to site

    #[test]
    #[ignore]
    fn test_insert_nonexisting_id() {
        let mut lseq = LSeq::<char, u64>::default();
        let actor = 100;

        // Insert A to [] (between BEGIN and END)
        insert_between_i_j(&mut lseq, None, None, 'A', actor);

        // Insert B to [A] (between BEGIN and <invalid id>)
        let seq = lseq.read().val;
        println!("SEQ [A]: {:?}", seq);
        assert_eq!(seq.len(), 1);

        let add_ctx = lseq.read_ctx().derive_add_ctx(actor);
        let op = lseq.insert('B', None, Some(Identifier::new(&[11])), add_ctx.clone());
        // should fail? will VClock help us here to know it's just an id we are not aware of yet??
        lseq.apply(op);
    }
}
