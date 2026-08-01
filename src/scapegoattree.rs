use core::alloc::Allocator;
use core::borrow::Borrow;
use core::cmp::{Ord, Ordering};
use core::fmt;
use std::alloc::Global;

impl<K, V> ScapeGoatTree<K, V> {
    pub fn new() -> Self {
        Self {
            tree: None,
            len: 0,
            max: 0,
            alloc: Global,
        }
    }
}

impl<K, V, A> ScapeGoatTree<K, V, A>
where
    K: Ord,
    A: Allocator,
{
    pub fn new_in(alloc: A) -> Self {
        Self {
            tree: None,
            len: 0,
            max: 0,
            alloc,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get<Q: ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord,
    {
        let mut cur = self.tree.as_deref();
        while let Some(tree) = cur {
            cur = match tree.key.borrow().cmp(key) {
                Ordering::Equal => return Some(&tree.value),
                Ordering::Greater => tree.left.as_deref(),
                Ordering::Less => tree.right.as_deref(),
            };
        }
        return None;
    }

    pub fn get_mut<Q: ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Ord,
    {
        let mut cur = self.tree.as_deref_mut();
        while let Some(tree) = cur {
            cur = match tree.key.borrow().cmp(key) {
                Ordering::Equal => return Some(&mut tree.value),
                Ordering::Greater => tree.left.as_deref_mut(),
                Ordering::Less => tree.right.as_deref_mut(),
            };
        }
        return None;
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V>
    where
        K: Ord,
        A: Clone,
    {
        let mut depth = 0;
        let mut cur = &mut self.tree;
        while let Some(tree) = cur {
            cur = match tree.key.borrow().cmp(&key) {
                Ordering::Equal => return Some(core::mem::replace(&mut tree.value, value)),
                Ordering::Greater => &mut tree.left,
                Ordering::Less => &mut tree.right,
            };
            depth += 1;
        }

        self.len += 1;
        self.max = usize::max(self.len, self.max);

        let ln_three_halfs: f64 = 1.5_f64.ln(); // i wish this was a constant
        let depth_max = ((self.len as f64).ln() / ln_three_halfs) as u32;
        let leaf = Box::new_in(
            BinaryTree {
                key,
                value,
                left: None,
                right: None,
            },
            self.alloc.clone(),
        );
        if depth <= depth_max {
            *cur = Some(leaf);
            return None;
        }

        // rebalance
        let mut cur = {
            let mut par = None;
            let mut cur = self.tree.take();
            while let Some(mut tree) = cur {
                cur = match tree.key.borrow().cmp(&leaf.key) {
                    Ordering::Equal => unreachable!("Rebalancing requires insertion of a key."),
                    Ordering::Greater => {
                        let cld = tree.left;
                        tree.left = par;
                        par = Some(tree);
                        cld
                    }
                    Ordering::Less => {
                        let cld = tree.right;
                        tree.right = par;
                        par = Some(tree);
                        cld
                    }
                };
            }
            par
        };
        // find scapegoat
        let (scapegoat, rest) = {
            let mut vine = Vine::from(leaf);
            let mut old;
            let mut new;
            while let Some(mut tree) = cur {
                (cur, old, new) = match tree.key.cmp(&vine.head.key) {
                    Ordering::Equal => unreachable!("Rebalancing requires insertion of a key."),
                    Ordering::Greater => {
                        let par = tree.left.take();
                        let new_vine = Vine::from(tree);
                        let old = vine.size;
                        let new = new_vine.size;
                        vine.concat(new_vine);
                        (par, old, new)
                    }
                    Ordering::Less => {
                        let par = tree.right.take();
                        let mut new_vine = Vine::from(tree);
                        let old = vine.size;
                        let new = new_vine.size;
                        new_vine.concat(vine);
                        vine = new_vine;
                        (par, old, new)
                    }
                };
                // found scapegoat
                if (old >> 1) > new {
                    break;
                }
            }
            (Into::<Box<BinaryTree<K, V, A>, A>>::into(vine), cur)
        };
        // repair top
        let mut cld = scapegoat;
        cur = rest;
        while let Some(mut tree) = cur {
            cur = match tree.key.cmp(&cld.key) {
                Ordering::Equal => unreachable!("Rebalancing requires insertion of a key."),
                Ordering::Greater => {
                    let par = tree.left;
                    tree.left = Some(cld);
                    cld = tree;
                    par
                }
                Ordering::Less => {
                    let par = tree.right;
                    tree.right = Some(cld);
                    cld = tree;
                    par
                }
            }
        }
        self.tree = Some(cld);
        // end rebalance
        None
    }
}

impl<K, V, A: Allocator> Vine<K, V, A> {
    fn concat(&mut self, other: Self) {
        unsafe {
            self.tail.as_mut().right = Some(other.head);
        }
        self.tail = other.tail;
        self.size += other.size;
    }
}

impl<K, V, A: Allocator> From<Box<BinaryTree<K, V, A>, A>> for Vine<K, V, A> {
    fn from(mut root: Box<BinaryTree<K, V, A>, A>) -> Self {
        while let Some(mut left) = root.left.take() {
            root.left = left.right.take();
            left.right = Some(root);
            root = left;
        }
        let mut par = root.as_mut();
        let mut size = 1;
        while let Some(mut cur) = par.right.take() {
            size += 1;
            while let Some(mut left) = cur.left.take() {
                cur.left = left.right.take();
                left.right = Some(cur);
                cur = left;
            }
            let next = par.right.insert(cur);
            par = next;
        }
        let tail = core::ptr::NonNull::from(par);
        Vine {
            head: root,
            tail,
            size,
        }
    }
}

struct Vine<K, V, A: Allocator> {
    head: Box<BinaryTree<K, V, A>, A>,
    tail: core::ptr::NonNull<BinaryTree<K, V, A>>,
    size: usize,
}

impl<K, V, A: Allocator> From<Vine<K, V, A>> for Box<BinaryTree<K, V, A>, A> {
    fn from(vine: Vine<K, V, A>) -> Self {
        let mut root = vine.head;
        let mut n = vine.size;
        let trimmed = 1 << n.ilog2();
        let uc = 1 + n - trimmed;
        if uc != trimmed {
            let mut i = 0;
            let mut k = 1;
            let mut m = 0;
            let mut par = &mut root;
            while let Some(mut cur) = par.right.take() {
                if let Some(mut right) = cur.right.take() {
                    if i == m {
                        m = k * n / uc;
                        k += 1;
                        cur.right = right.left.take();
                        right.left = Some(cur);
                        cur = right;
                        i += 2;
                    } else {
                        cur.right = Some(right);
                        i += 1;
                    }
                    let next = par.right.insert(cur);
                    par = next;
                    continue;
                }
                par.right = Some(cur);
                break;
            }
            n -= uc;
        }
        while n > 1 {
            root = {
                let mut right = root.right.take().expect("n > 1");
                root.right = right.left.take();
                right.left = Some(root);
                right
            };
            let m = n >> 1;
            let mut par = &mut root;
            for _ in 1..m {
                let mut cur = par.right.take().expect("i < m");
                let mut right = cur.right.take().expect("i < m");
                cur.right = right.left.take();
                right.left = Some(cur);
                let next = par.right.insert(right);
                par = next;
            }
            n = m;
        }
        root
    }
}

struct BinaryTree<K, V, A: Allocator = Global> {
    key: K,
    value: V,
    left: Option<Box<BinaryTree<K, V, A>, A>>,
    right: Option<Box<BinaryTree<K, V, A>, A>>,
}

pub struct ScapeGoatTree<K, V, A: Allocator = Global> {
    tree: Option<Box<BinaryTree<K, V, A>, A>>,
    len: usize,
    max: usize,
    alloc: A,
}

impl<K, V, A> fmt::Debug for BinaryTree<K, V, A>
where
    K: fmt::Debug,
    V: fmt::Debug,
    A: Allocator,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(")?;

        if let Some(left) = &self.left {
            write!(f, "{:?} <- ", left)?;
        }

        write!(f, "{:?}:{:?}", self.key, self.value)?;
        if let Some(right) = &self.right {
            write!(f, " -> {:?}", right)?;
        }
        write!(f, ")")
    }
}

impl<K, V, A> fmt::Display for BinaryTree<K, V, A>
where
    K: fmt::Display,
    V: fmt::Display,
    A: Allocator,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(")?;

        if let Some(left) = &self.left {
            write!(f, "{left} <- ")?;
        }

        write!(f, "{}:{}", self.key, self.value)?;
        if let Some(right) = &self.right {
            write!(f, " -> {right}")?;
        }
        write!(f, ")")
    }
}

impl<K, V, A> fmt::Display for ScapeGoatTree<K, V, A>
where
    K: fmt::Display,
    V: fmt::Display,
    A: Allocator,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "ScapeGoatTree {{")?;
        writeln!(f, "  len: {},", self.len)?;
        writeln!(f, "  max: {},", self.max)?;

        write!(f, "  tree: ")?;
        match &self.tree {
            Some(tree) => write!(f, "{tree}")?,
            None => write!(f, "∅")?,
        }

        writeln!(f)?;
        write!(f, "}}")
    }
}
