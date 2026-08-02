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
        None
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
        None
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
        let leaf = Box::new_in(
            BinaryTree {
                key,
                value,
                left: None,
                right: None,
            },
            self.alloc.clone(),
        );

        const INV_LN_THREE_HALFS: f64 = 2.4663034623764317;
        let depth_max = (INV_LN_THREE_HALFS * (self.len as f64).ln()) as u32;
        if depth <= depth_max {
            *cur = Some(leaf);
            return None;
        }

        // rebalance
        let mut cur = reverse_path(self.tree.take(), &leaf.key);
        // find scapegoat
        let (scapegoat, rest) = {
            let mut vine = Vine::from(leaf);
            let mut old;
            let mut new = 1;
            while let Some(mut tree) = cur {
                match tree.key.cmp(&vine.head.key) {
                    Ordering::Equal => unreachable!("Rebalancing requires insertion of a key."),
                    Ordering::Greater => {
                        cur = tree.left.take();
                        vine.concat(Vine::from(tree));
                    }
                    Ordering::Less => {
                        cur = tree.right.take();
                        let mut new_vine = Vine::from(tree);
                        new_vine.concat(vine);
                        vine = new_vine;
                    }
                };
                old = new;
                new = vine.size - old;
                // found scapegoat
                if old > (new << 1) {
                    break;
                }
            }
            (Into::into(vine), cur)
        };
        self.tree = Some(restore_path(scapegoat, rest));
        // end rebalance
        None
    }

    fn remove_rebuild(&mut self) {
        self.len -= 1;
        if 3 * self.len < 2 * self.max
            && let Some(x) = self.tree.take()
        {
            self.max = self.len;
            self.tree = Some(Vine::from(x).into());
        }
    }

    pub fn pop_first(&mut self) -> Option<(K, V)> {
        let mut cur = self.tree.take()?;
        let mut par = &mut self.tree;
        while let Some(left) = cur.left.take() {
            let next = par.insert(cur);
            par = &mut next.left;
            cur = left;
        }
        *par = cur.right;
        self.remove_rebuild();
        Some((cur.key, cur.value))
    }

    pub fn pop_last(&mut self) -> Option<(K, V)> {
        let mut cur = self.tree.take()?;
        let mut par = &mut self.tree;
        while let Some(right) = cur.right.take() {
            let next = par.insert(cur);
            par = &mut next.right;
            cur = right;
        }
        *par = cur.left;
        self.remove_rebuild();
        Some((cur.key, cur.value))
    }

    pub fn remove<Q: ?Sized>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Ord,
    {
        self.remove_entry(key).map(|(_, x)| x)
    }

    pub fn remove_entry<Q: ?Sized>(&mut self, key: &Q) -> Option<(K, V)>
    where
        K: Borrow<Q>,
        Q: Ord,
    {
        let mut cur = self.tree.take()?;
        let mut par = &mut self.tree;
        loop {
            match cur.key.borrow().cmp(key) {
                Ordering::Equal => break,
                Ordering::Greater => {
                    let Some(left) = cur.left.take() else {
                        *par = Some(cur);
                        return None;
                    };
                    let next = par.insert(cur);
                    par = &mut next.left;
                    cur = left;
                }
                Ordering::Less => {
                    let Some(right) = cur.right.take() else {
                        *par = Some(cur);
                        return None;
                    };
                    let next = par.insert(cur);
                    par = &mut next.right;
                    cur = right;
                }
            }
        }
        match (cur.left.take(), cur.right.take()) {
            (None, None) => {}
            (l @ Some(_), None) => *par = l,
            (None, r @ Some(_)) => *par = r,
            (l @ Some(_), Some(mut r_cur)) => {
                let mut r = None;
                let mut r_par = &mut r;
                while let Some(left) = r_cur.left.take() {
                    let next = r_par.insert(r_cur);
                    r_par = &mut next.left;
                    r_cur = left;
                }
                *r_par = r_cur.right;
                r_cur.left = l;
                r_cur.right = r;
                *par = Some(r_cur);
            }
        }
        self.remove_rebuild();
        Some((cur.key, cur.value))
    }
}

fn reverse_path<Q: ?Sized, K, V, A>(
    mut cur: Option<Box<BinaryTree<K, V, A>, A>>,
    key: &Q,
) -> Option<Box<BinaryTree<K, V, A>, A>>
where
    K: Borrow<Q>,
    Q: Ord,
    A: Allocator,
{
    let mut cld = None;
    while let Some(mut tree) = cur {
        match tree.key.borrow().cmp(&key) {
            Ordering::Equal => unreachable!("Binary tree has distinct keys."),
            Ordering::Greater => {
                cur = tree.left;
                tree.left = cld;
            }
            Ordering::Less => {
                cur = tree.right;
                tree.right = cld;
            }
        };
        cld = Some(tree);
    }
    cld
}

fn restore_path<K: Ord, V, A: Allocator>(
    mut cld: Box<BinaryTree<K, V, A>, A>,
    mut cur: Option<Box<BinaryTree<K, V, A>, A>>,
) -> Box<BinaryTree<K, V, A>, A> {
    while let Some(mut tree) = cur {
        match tree.key.cmp(&cld.key) {
            Ordering::Equal => unreachable!("Binary tree has distinct keys."),
            Ordering::Greater => {
                cur = tree.left;
                tree.left = Some(cld);
            }
            Ordering::Less => {
                cur = tree.right;
                tree.right = Some(cld);
            }
        }
        cld = tree;
    }
    cld
}

impl<K, V, A: Allocator> Vine<K, V, A> {
    fn concat(&mut self, other: Self) {
        unsafe {
            *self.tail.as_mut() = Some(other.head);
        }
        self.tail = other.tail;
        self.size += other.size;
    }
}

impl<K, V, A: Allocator> From<Box<BinaryTree<K, V, A>, A>> for Vine<K, V, A> {
    fn from(mut root: Box<BinaryTree<K, V, A>, A>) -> Self {
        while let Some(mut left) = root.left {
            root.left = left.right;
            left.right = Some(root);
            root = left;
        }
        let mut par = &mut root.right;
        let mut size = 1;
        while let Some(mut cur) = par.take() {
            size += 1;
            while let Some(mut left) = cur.left {
                cur.left = left.right;
                left.right = Some(cur);
                cur = left;
            }
            let next = par.insert(cur);
            par = &mut next.right;
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
    tail: core::ptr::NonNull<Option<Box<BinaryTree<K, V, A>, A>>>,
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
                let Some(mut right) = cur.right else {
                    par.right = Some(cur);
                    break;
                };
                if i == m {
                    m = k * n / uc;
                    k += 1;
                    cur.right = right.left;
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
            n -= uc;
        }
        while n > 1 {
            root = {
                let mut right = root.right.expect("n > 1");
                root.right = right.left;
                right.left = Some(root);
                right
            };
            let m = n >> 1;
            let mut par = &mut root;
            for _ in 1..m {
                let mut cur = par.right.take().expect("i < m");
                let mut right = cur.right.expect("i < m");
                cur.right = right.left;
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
