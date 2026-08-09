use core::alloc::{Allocator, Layout};
use core::borrow::Borrow;
use core::cmp::{Ord, Ordering};
use core::fmt;
use core::ptr::NonNull;
use std::alloc::{Global, handle_alloc_error};

impl<K, V> ScapeGoatTreeMap<K, V> {
    pub fn new() -> Self {
        Self {
            tree: None,
            len: 0,
            max: 0,
            alloc: Global,
        }
    }
}

impl<K, V, A> ScapeGoatTreeMap<K, V, A>
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

    pub fn clear(&mut self) {
        let mut cur = self.tree.take().map(|x| Vine::from(x).head);
        while let Some(tree) = cur {
            cur = unsafe { tree.read() }.right;
            unsafe { self.alloc.deallocate(tree.cast(), node_layout::<K, V>()) };
        }
        self.len = 0;
        self.max = 0;
    }

    pub fn contains_key<Q: ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q> + Ord,
        Q: Ord,
    {
        self.get(key).is_some()
    }

    pub fn get<Q: ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord,
    {
        let mut cur = self.tree.as_ref();
        while let Some(tree) = cur.map(|ptr| unsafe { ptr.as_ref() }) {
            cur = match tree.key.borrow().cmp(key) {
                Ordering::Equal => return Some(&tree.value),
                Ordering::Greater => tree.left.as_ref(),
                Ordering::Less => tree.right.as_ref(),
            };
        }
        None
    }

    pub fn get_mut<Q: ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Ord,
    {
        let mut cur = self.tree.as_mut();
        while let Some(tree) = cur.map(|ptr| unsafe { ptr.as_mut() }) {
            cur = match tree.key.borrow().cmp(key) {
                Ordering::Equal => return Some(&mut tree.value),
                Ordering::Greater => tree.left.as_mut(),
                Ordering::Less => tree.right.as_mut(),
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
        while let Some(tree) = cur.map(|mut ptr| unsafe { ptr.as_mut() }) {
            cur = match tree.key.borrow().cmp(&key) {
                Ordering::Equal => return Some(core::mem::replace(&mut tree.value, value)),
                Ordering::Greater => &mut tree.left,
                Ordering::Less => &mut tree.right,
            };
            depth += 1;
        }

        self.len += 1;
        self.max = usize::max(self.len, self.max);
        let leaf = self
            .alloc
            .allocate(node_layout::<K, V>())
            .unwrap_or_else(|_| handle_alloc_error(node_layout::<K, V>()))
            .cast();
        unsafe {
            leaf.write(Node {
                key,
                value,
                left: None,
                right: None,
            })
        }

        const INV_LN_THREE_HALFS: f64 = 2.4663034623764317;
        let depth_max = (INV_LN_THREE_HALFS * (self.len as f64).ln()) as u32;
        if depth <= depth_max {
            *cur = Some(leaf);
            return None;
        }

        // rebalance
        let cur = reverse_path(self.tree.take(), &unsafe { leaf.as_ref() }.key);
        let (scapegoat, rest) = find_scapegoat(leaf, cur);
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

    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        let mut cur = self.tree.map(|ptr| unsafe { ptr.as_ref() })?;
        while let Some(left) = cur.left {
            cur = unsafe { left.as_ref() };
        }
        Some((&cur.key, &cur.value))
    }

    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        let mut cur = self.tree.map(|ptr| unsafe { ptr.as_ref() })?;
        while let Some(right) = cur.right.as_ref() {
            cur = unsafe { right.as_ref() };
        }
        Some((&cur.key, &cur.value))
    }

    pub fn pop_first(&mut self) -> Option<(K, V)> {
        let mut cur = self.tree.take()?;
        let mut par = &mut self.tree;
        while let Some(left) = unsafe { cur.as_mut() }.left.take() {
            let next = par.insert(cur);
            par = &mut unsafe { next.as_mut() }.left;
            cur = left;
        }
        let cur = unsafe {
            let tmp = cur.read();
            self.alloc.deallocate(cur.cast(), node_layout::<K, V>());
            tmp
        };
        *par = cur.right;
        self.remove_rebuild();
        Some((cur.key, cur.value))
    }

    pub fn pop_last(&mut self) -> Option<(K, V)> {
        let mut cur = self.tree.take()?;
        let mut par = &mut self.tree;
        while let Some(right) = unsafe { cur.as_mut() }.right.take() {
            let next = par.insert(cur);
            par = &mut unsafe { next.as_mut() }.right;
            cur = right;
        }
        let cur = unsafe {
            let tmp = cur.read();
            self.alloc.deallocate(cur.cast(), node_layout::<K, V>());
            tmp
        };
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
            let cur_mut = unsafe { cur.as_mut() };
            match cur_mut.key.borrow().cmp(key) {
                Ordering::Equal => break,
                Ordering::Greater => {
                    let Some(left) = cur_mut.left.take() else {
                        *par = Some(cur);
                        return None;
                    };
                    let next = par.insert(cur);
                    let next_mut = unsafe { next.as_mut() };
                    par = &mut next_mut.left;
                    cur = left;
                }
                Ordering::Less => {
                    let Some(right) = cur_mut.right.take() else {
                        *par = Some(cur);
                        return None;
                    };
                    let next = par.insert(cur);
                    let next_mut = unsafe { next.as_mut() };
                    par = &mut next_mut.right;
                    cur = right;
                }
            }
        }
        let mut cur = unsafe {
            let tmp = cur.read();
            self.alloc.deallocate(cur.cast(), node_layout::<K, V>());
            tmp
        };
        match (cur.left.take(), cur.right.take()) {
            (None, None) => {}
            (l @ Some(_), None) => *par = l,
            (None, r @ Some(_)) => *par = r,
            (l @ Some(_), Some(mut r_cur)) => {
                let mut r = None;
                let mut r_par = &mut r;
                while let Some(left) = unsafe { r_cur.as_mut() }.left.take() {
                    let next = r_par.insert(r_cur);
                    let next_mut = unsafe { next.as_mut() };
                    r_par = &mut next_mut.left;
                    r_cur = left;
                }
                let r_cur_mut = unsafe { r_cur.as_mut() };
                *r_par = r_cur_mut.right;
                r_cur_mut.left = l;
                r_cur_mut.right = r;
                *par = Some(r_cur);
            }
        }
        self.remove_rebuild();
        Some((cur.key, cur.value))
    }
}

fn reverse_path<Q: ?Sized, K, V>(
    mut cur: Option<NonNull<Node<K, V>>>,
    key: &Q,
) -> Option<NonNull<Node<K, V>>>
where
    K: Borrow<Q>,
    Q: Ord,
{
    let mut cld = None;
    while let Some(mut tree) = cur {
        let tree_mut = unsafe { tree.as_mut() };
        match tree_mut.key.borrow().cmp(&key) {
            Ordering::Equal => unreachable!("Binary tree has distinct keys."),
            Ordering::Greater => {
                cur = tree_mut.left;
                tree_mut.left = cld;
            }
            Ordering::Less => {
                cur = tree_mut.right;
                tree_mut.right = cld;
            }
        };
        cld = Some(tree);
    }
    cld
}

fn find_scapegoat<K: Ord, V>(
    leaf: NonNull<Node<K, V>>,
    mut cur: Option<NonNull<Node<K, V>>>,
) -> (NonNull<Node<K, V>>, Option<NonNull<Node<K, V>>>) {
    let mut vine = Vine::from(leaf);
    let mut old = 0;
    let mut new = 1;
    while let Some(mut tree) = cur {
        let tree_mut = unsafe { tree.as_mut() };
        match tree_mut.key.cmp(&unsafe { vine.head.as_ref() }.key) {
            Ordering::Equal => unreachable!("Binary tree has distinct keys."),
            Ordering::Greater => {
                cur = tree_mut.left.take();
                vine.concat(Vine::from(tree));
            }
            Ordering::Less => {
                cur = tree_mut.right.take();
                let mut new_vine = Vine::from(tree);
                new_vine.concat(vine);
                vine = new_vine;
            }
        };
        old += new;
        new = vine.size - old;
        // found scapegoat, i.e. |T_child| > 2/3 |T|
        if old > (new << 1) {
            break;
        }
    }
    (Into::into(vine), cur)
}

fn restore_path<K: Ord, V>(
    mut cld: NonNull<Node<K, V>>,
    mut cur: Option<NonNull<Node<K, V>>>,
) -> NonNull<Node<K, V>> {
    let key = unsafe { &cld.as_ref().key };
    while let Some(mut tree) = cur {
        let tree_mut = unsafe { tree.as_mut() };
        match tree_mut.key.cmp(key) {
            Ordering::Equal => unreachable!("Binary tree has distinct keys."),
            Ordering::Greater => {
                cur = tree_mut.left;
                tree_mut.left = Some(cld);
            }
            Ordering::Less => {
                cur = tree_mut.right;
                tree_mut.right = Some(cld);
            }
        }
        cld = tree;
    }
    cld
}

impl<K, V> Vine<K, V> {
    fn concat(&mut self, other: Self) {
        unsafe {
            self.tail.write(Some(other.head));
        }
        self.tail = other.tail;
        self.size += other.size;
    }
}

impl<K, V> From<NonNull<Node<K, V>>> for Vine<K, V> {
    fn from(mut root: NonNull<Node<K, V>>) -> Self {
        unsafe {
            while let Some(mut left) = root.as_ref().left {
                root.as_mut().left = left.as_ref().right;
                left.as_mut().right = Some(root);
                root = left;
            }
        }

        let mut par = &mut unsafe { root.as_mut() }.right;
        let mut size = 1;
        while let Some(mut cur) = par.take() {
            size += 1;
            unsafe {
                while let Some(mut left) = cur.as_ref().left {
                    cur.as_mut().left = left.as_ref().right;
                    left.as_mut().right = Some(cur);
                    cur = left;
                }
            }
            let next = par.insert(cur);
            par = &mut unsafe { next.as_mut() }.right;
        }
        let tail = NonNull::from(par);
        Vine {
            head: root,
            tail,
            size,
        }
    }
}

impl<K, V> From<Vine<K, V>> for NonNull<Node<K, V>> {
    fn from(vine: Vine<K, V>) -> Self {
        let mut root = vine.head;
        let mut n = vine.size;
        let trimmed = n.isolate_highest_one();
        let uc = 1 + n - trimmed;
        if uc != trimmed {
            let mut i = 0;
            let mut k = 1;
            let mut m = 0;
            let mut par = &mut root;
            while let Some(mut cur) = unsafe { par.as_mut().right }.take() {
                let cur_mut = unsafe { cur.as_mut() };
                let par_mut = unsafe { par.as_mut() };
                let Some(mut right) = cur_mut.right else {
                    par_mut.right = Some(cur);
                    break;
                };
                let right_mut = unsafe { right.as_mut() };
                if i == m {
                    m = k * n / uc;
                    k += 1;
                    cur_mut.right = right_mut.left;
                    right_mut.left = Some(cur);
                    cur = right;
                    i += 2;
                } else {
                    cur_mut.right = Some(right);
                    i += 1;
                }
                let next = par_mut.right.insert(cur);
                par = next;
                continue;
            }
            n -= uc;
        }
        while n > 1 {
            root = {
                let root_mut = unsafe { root.as_mut() };
                let mut right = root_mut.right.expect("n > 1");
                let right_mut = unsafe { right.as_mut() };
                root_mut.right = right_mut.left;
                right_mut.left = Some(root);
                right
            };
            let m = n >> 1;
            let mut par = &mut root;
            for _ in 1..m {
                let par_mut = unsafe { par.as_mut() };
                let mut cur = par_mut.right.take().expect("i < m");
                let cur_mut = unsafe { cur.as_mut() };
                let mut right = cur_mut.right.expect("i < m");
                let right_mut = unsafe { right.as_mut() };
                cur_mut.right = right_mut.left;
                right_mut.left = Some(cur);
                let next = par_mut.right.insert(right);
                par = next;
            }
            n = m;
        }
        root
    }
}

pub const fn node_layout<K, V>() -> Layout {
    Layout::new::<Node<K, V>>()
}

struct Vine<K, V> {
    head: NonNull<Node<K, V>>,
    tail: NonNull<Option<NonNull<Node<K, V>>>>,
    size: usize,
}

struct Node<K, V> {
    key: K,
    value: V,
    left: Option<NonNull<Node<K, V>>>,
    right: Option<NonNull<Node<K, V>>>,
}

#[derive(Debug)]
pub struct ScapeGoatTreeMap<K, V, A: Allocator = Global> {
    tree: Option<NonNull<Node<K, V>>>,
    len: usize,
    max: usize,
    alloc: A,
}

impl<K, V> Node<K, V>
where
    K: fmt::Display,
    V: fmt::Display,
{
    fn fmt_pretty(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(right) = self.right.map(|ptr| unsafe { ptr.as_ref() }) {
            right.fmt_pretty_right(f, String::from("   "))?;
        }
        writeln!(f, "{}:{}", self.key, self.value)?;
        if let Some(left) = self.left.map(|ptr| unsafe { ptr.as_ref() }) {
            left.fmt_pretty_left(f, String::from("   "))?;
        }
        Ok(())
    }

    fn fmt_pretty_right(&self, f: &mut fmt::Formatter<'_>, prefix: String) -> fmt::Result {
        if let Some(right) = self.right.map(|ptr| unsafe { ptr.as_ref() }) {
            right.fmt_pretty_right(f, prefix.clone() + "    ")?;
        }
        writeln!(f, "{} ˏ——— {}:{}", prefix, self.key, self.value)?;
        if let Some(left) = self.left.map(|ptr| unsafe { ptr.as_ref() }) {
            left.fmt_pretty_left(f, prefix + "⎹   ")?;
        }
        Ok(())
    }

    fn fmt_pretty_left(&self, f: &mut fmt::Formatter<'_>, prefix: String) -> fmt::Result {
        if let Some(right) = self.right.map(|ptr| unsafe { ptr.as_ref() }) {
            right.fmt_pretty_right(f, prefix.clone() + "⎹   ")?;
        }
        writeln!(f, "{} `——— {}:{}", prefix, self.key, self.value)?;
        if let Some(left) = self.left.map(|ptr| unsafe { ptr.as_ref() }) {
            left.fmt_pretty_left(f, prefix + "    ")?;
        }
        Ok(())
    }
}

impl<K, V, A: Allocator> Drop for ScapeGoatTreeMap<K, V, A> {
    fn drop(&mut self) {
        let mut cur = self.tree.take().map(|x| Vine::from(x).head);
        while let Some(tree) = cur {
            cur = unsafe { tree.read() }.right;
            unsafe { self.alloc.deallocate(tree.cast(), node_layout::<K, V>()) };
        }
    }
}

impl<K, V> fmt::Display for Node<K, V>
where
    K: fmt::Display,
    V: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            return self.fmt_pretty(f);
        }
        write!(f, "(")?;
        if let Some(left) = self.left.map(|ptr| unsafe { ptr.as_ref() }) {
            write!(f, "{left} ← ")?;
        }
        write!(f, "{}:{}", self.key, self.value)?;
        if let Some(right) = self.right.map(|ptr| unsafe { ptr.as_ref() }) {
            write!(f, " → {right}")?;
        }
        write!(f, ")")
    }
}

impl<K, V> FromIterator<(K, V)> for ScapeGoatTreeMap<K, V>
where
    K: Ord,
{
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut tree = Self::new();
        for (key, value) in iter {
            tree.insert(key, value);
        }
        tree
    }
}

impl<K, V, A> fmt::Display for ScapeGoatTreeMap<K, V, A>
where
    K: fmt::Display,
    V: fmt::Display,
    A: Allocator,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "ScapeGoatTree {{\n  len: {}, max: {}\n   tree:",
            self.len, self.max
        )?;
        match self.tree.map(|ptr| unsafe { ptr.as_ref() }) {
            Some(tree) => tree.fmt(f)?,
            None => f.write_str("∅")?,
        }
        write!(f, "\n}}")
    }
}
