use crdts::lseq::{LSeq, Op};
use crdts::CmRDT;
use rand::distributions::Alphanumeric;
use rand::Rng;

type SiteId = u32;
#[derive(Debug, Clone)]
struct OperationList(pub Vec<Op<char, SiteId>>);

use quickcheck::{Arbitrary, Gen, TestResult};

impl Arbitrary for OperationList {
    fn arbitrary<G: Gen>(g: &mut G) -> OperationList {
        let size = {
            let s = g.size();
            if s == 0 {
                0
            } else {
                g.gen_range(0, s)
            }
        };

        let mut site1 = LSeq::new(g.gen());
        let ops = (0..size)
            .filter_map(|_| {
                if g.gen() || site1.len() == 0 {
                    site1.delete_index(g.gen_range(0, site1.len() + 1))
                } else {
                    site1.delete_index(g.gen_range(0, site1.len()))
                }
            })
            .collect();
        OperationList(ops)
    }
    // implement shrinking ://
}

#[test]
fn test_new() {
    let site1: LSeq<char, SiteId> = LSeq::new(0);
    assert_eq!(site1.len(), 0);
    assert!(site1.is_empty());
}

#[test]
fn test_is_empty() {
    let mut site1 = LSeq::new(0);
    assert!(site1.is_empty());

    site1.insert_index(0, 'a');
    assert!(!site1.is_empty());
}

#[test]
fn test_append() {
    let mut site1 = LSeq::new(0);
    assert!(site1.is_empty());

    site1.append('a');
    site1.append('b');
    site1.append('c');

    assert_eq!(site1.iter().collect::<String>(), "abc");
}

#[test]
fn test_out_of_order_inserts() {
    let mut site1 = LSeq::new(0);
    site1.insert_index(0, 'a');
    site1.insert_index(1, 'c');
    site1.insert_index(1, 'b');

    assert_eq!(site1.iter().collect::<String>(), "abc");
}

#[test]
fn test_append_mixed_with_inserts() {
    let mut site1 = LSeq::new(0);
    site1.append('a');
    site1.insert_index(0, 'b');
    site1.append('c');
    site1.insert_index(1, 'd');

    assert_eq!(site1.iter().collect::<String>(), "bdac");
}

#[test]
fn test_delete_of_index() {
    let mut site1 = LSeq::new(0);
    site1.insert_index(0, 'a');
    site1.insert_index(1, 'b');
    assert_eq!(site1.iter().collect::<String>(), "ab");

    site1.delete_index(0);
    assert_eq!(site1.iter().collect::<String>(), "b");
}

#[test]
fn test_get() {
    let mut site1 = LSeq::new(0);
    site1.append('a');
    site1.append('b');

    assert_eq!(site1.get(0), Some(&'a'));
    assert_eq!(site1.get(1), Some(&'b'));
}

#[test]
fn test_worst_case_inserts() {
    // by inserting always at the middle of the array, we grow the exponential tree beyond the
    // max depth very quickly. This is not looking too good, we need to tune some params.

    let mut site = LSeq::new(0);
    // let mut site2 = LSeq::new((1));

    let n = 86; // maximum reliable number of inserts we can do in this worst case example
    for _ in 0..n {
        let i = site.len() / 2;
        println!("inserting {}/{}", i, site.len());
        site.insert_index(i, 'a');
    }
    assert_eq!(site.len(), n);
}

#[test]
fn test_insert_followed_by_deletes() {
    let mut rng = rand::thread_rng();

    let mut s1 = rng.sample_iter(Alphanumeric);

    let mut site1 = LSeq::new(0);
    let mut site2 = LSeq::new(1);

    for _ in 0..5000 {
        let c = s1.next().unwrap();
        let ix = rng.gen_range(0, site1.len() + 1);
        let insert_op = site1.insert_index(ix, c);
        site2.apply(insert_op);

        let delete_op =
            site2
                .delete_index(ix)
                .expect(&format!("ix@{} was out of bounds@{}", ix, site2.len()));
        site1.apply(delete_op);
    }

    assert!(
        site1.is_empty(),
        "site1 was not empty: {}",
        site1.iter().collect::<String>()
    );
    assert!(
        site2.is_empty(),
        "site2 was not empty: {}",
        site2.iter().collect::<String>()
    );
}

#[test]
fn test_mutual_insert_qc1() {
    let mut site0 = LSeq::new(0);
    let mut site1 = LSeq::new(1);
    let plan = vec![
        (8, 24, false),
        (23, 1, true),
        (93, 94, false),
        (68, 30, false),
        (37, 27, true),
    ];

    for (elem, idx, source_is_site0) in plan {
        let (source, replica) = if source_is_site0 {
            (&mut site0, &mut site1)
        } else {
            (&mut site1, &mut site0)
        };
        let i = idx % (source.len() + 1);
        println!("{:?} inserting {} @ {}", source.actor(), elem, i);
        let op = source.insert_index(i, elem);
        replica.apply(op);
    }

    assert_eq!(
        site0.iter().collect::<Vec<_>>(),
        site1.iter().collect::<Vec<_>>()
    );
}

quickcheck! {
    fn prop_mutual_inserting(plan: Vec<(u8, usize, bool)>) -> bool {
        let mut site0 = LSeq::new(0);
        let mut site1 = LSeq::new(1);
        for (elem, idx, source_is_site0) in plan {
            let (source, replica) = if source_is_site0 {
                (&mut site0, &mut site1)
            } else {
                (&mut site1, &mut site0)
            };
            let i = idx % (source.len() + 1);
            let op = source.insert_index(i, elem);
            replica.apply(op);

        }

        assert_eq!(site0.iter().collect::<Vec<_>>(), site1.iter().collect::<Vec<_>>());
        true
    }

    fn prop_inserts_and_deletes(op1: OperationList, op2: OperationList) -> TestResult {
        let mut rng = quickcheck::StdThreadGen::new(1000);
        let mut op1 = op1.0.into_iter();
        let mut op2 = op2.0.into_iter();

        let mut site1 = LSeq::new(0);
        let mut site2 = LSeq::new(1);

        let mut s1_empty = false;
        let mut s2_empty = false;
        while !s1_empty && !s2_empty {
            if rng.gen() {
                match op1.next() {
                    Some(o) => {
                        site1.apply(o.clone());
                        site2.apply(o);
                    }
                    None => {
                        s1_empty = true;
                    }
                }
            } else {
                match op2.next() {
                    Some(o) => {
                        site1.apply(o.clone());
                        site2.apply(o);
                    }
                    None => {
                        s2_empty = true;
                    }
                }
            }
        }

        let site1_text = site1.iter().collect::<String>();
        let site2_text = site2.iter().collect::<String>();

        TestResult::from_bool(site1_text == site2_text)
    }

    fn prop_ops_are_idempotent(ops: OperationList) -> TestResult {
        let mut site1 = LSeq::new(0);
        let mut site2 = LSeq::new(1);

        for op in ops.0.into_iter() {
            // Apply the same op twice to site1
            site1.apply(op.clone());
            site1.apply(op.clone());

            // But only apply that op once to site2
            site2.apply(op);
        }

        let site1_text = site1.iter().collect::<String>();
        let site2_text = site2.iter().collect::<String>();

        TestResult::from_bool(site1_text == site2_text)
    }

    fn prop_len_is_proportional_to_ops(oplist: OperationList) -> TestResult {
        let mut expected_len = 0;
        let mut site1 = LSeq::new(0);

        for op in oplist.0.into_iter() {
            match op {
                Op::Insert { .. } => expected_len += 1,
                Op::Delete { .. } => expected_len -= 1,
            };
            site1.apply(op);
        }

        TestResult::from_bool(site1.len() == expected_len)
    }
}
