use std::borrow::Borrow;
use std::fmt;
use std::mem;

use unreachable::UncheckedOptionExt;

use iter::{Iter, IterMut, IntoIter};
use sparse::Sparse;
use util::{nybble_index, nybble_mismatch};

// A leaf in the trie.
pub struct Leaf<K: ToOwned, V> {
    pub key: K::Owned,
    pub val: V,
}


impl<K: ToOwned, V: Clone> Clone for Leaf<K, V> {
    #[inline]
    fn clone(&self) -> Self {
        Leaf {
            key: self.key.borrow().to_owned(),
            val: self.val.clone(),
        }
    }
}


impl<K: ToOwned + Borrow<[u8]>, V: PartialEq> PartialEq for Leaf<K, V> {
    #[inline]
    fn eq(&self, rhs: &Leaf<K, V>) -> bool {
        self.key_slice() == rhs.key_slice()
    }
}


impl<K: ToOwned, V> Leaf<K, V> {
    #[inline]
    pub fn new(key: K, val: V) -> Leaf<K, V> {
        Leaf {
            key: key.to_owned(),
            val,
        }
    }
}


impl<K: ToOwned + Borrow<[u8]>, V> Leaf<K, V> {
    #[inline]
    pub fn key_slice(&self) -> &[u8] {
        self.key.borrow().borrow()
    }
}


// A branch node in the QP-trie. It contains up to 17 entries, only 16 of which may actually be
// other branches - the 0th entry, if it exists in the sparse array, is the "head" of the branch,
// containing a key/value pair corresponding to the leaf which would otherwise occupy the location
// of the branch in the trie.
pub struct Branch<K: ToOwned, V> {
    // The nybble that this `Branch` cares about. Entries in the `entries` sparse array correspond
    // to different values of the nybble at the choice point for given keys.
    choice: usize,
    entries: Sparse<Node<K, V>>,
}


impl<K: ToOwned + Borrow<[u8]>, V: PartialEq> PartialEq for Branch<K, V> {
    #[inline]
    fn eq(&self, rhs: &Branch<K, V>) -> bool {
        self.choice == rhs.choice && self.entries == rhs.entries
    }
}


impl<K: ToOwned, V: Clone> Clone for Branch<K, V> {
    #[inline]
    fn clone(&self) -> Self {
        Branch {
            choice: self.choice,
            entries: self.entries.clone(),
        }
    }
}


impl<K: ToOwned, V: fmt::Debug> fmt::Debug for Branch<K, V>
where
    K::Owned: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Branch")
            .field("choice", &self.choice)
            .field("entries", &self.entries)
            .finish()
    }
}


impl<K: ToOwned + Borrow<[u8]>, V> Branch<K, V> {
    // Create an empty `Branch` with the given choice point.
    #[inline]
    pub fn new(choice: usize) -> Branch<K, V> {
        Branch {
            choice,
            entries: Sparse::new(),
        }
    }


    // Return the nybble index corresponding to the branch's choice point in the given key.
    #[inline]
    pub fn index(&self, key: &[u8]) -> u8 {
        nybble_index(self.choice, key)
    }


    // Returns true if and only if the `Branch` has only one child. This is used for determining
    // whether or not to replace a branch with its only child.
    #[inline]
    pub fn is_singleton(&self) -> bool {
        self.entries.len() == 1
    }


    #[inline]
    pub fn has_entry(&self, index: u8) -> bool {
        self.entries.contains(index)
    }


    #[inline]
    pub fn entry_mut(&mut self, index: u8) -> &mut Node<K, V> {
        let entry = self.entries.get_mut(index);
        debug_assert!(entry.is_some());
        unsafe { entry.unchecked_unwrap() }
    }


    // Get the child node corresponding to the given key.
    #[inline]
    pub fn child(&self, key: &[u8]) -> Option<&Node<K, V>> {
        self.entries.get(nybble_index(self.choice, key.borrow()))
    }


    // Mutable version of `Branch::child`.
    #[inline]
    pub fn child_mut(&mut self, key: &[u8]) -> Option<&mut Node<K, V>> {
        self.entries.get_mut(
            nybble_index(self.choice, key.borrow()),
        )
    }


    // Immutably borrow the leaf for the given key, if it exists, mutually recursing through
    // `Node::get`.
    #[inline]
    pub fn get(&self, key: &[u8]) -> Option<&Leaf<K, V>> {
        match self.child(key.borrow()) {
            Some(child) => child.get(key),
            None => None,
        }
    }


    // Mutably borrow the value for the given key, if it exists, mutually recursing through
    // `Node::get_mut`.
    #[inline]
    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut Leaf<K, V>> {
        self.child_mut(key.borrow()).and_then(
            |node| node.get_mut(key),
        )
    }


    // Retrieve the node which contains the exemplar. This does not recurse and return the actual
    // exemplar - just the node which might be or contain it.
    #[inline]
    pub fn exemplar(&self, key: &[u8]) -> &Node<K, V> {
        self.entries.get_or_any(
            nybble_index(self.choice, key.borrow()),
        )
    }


    // As `Branch::exemplar` but for mutable borrows.
    #[inline]
    pub fn exemplar_mut(&mut self, key: &[u8]) -> &mut Node<K, V> {
        self.entries.get_or_any_mut(
            nybble_index(self.choice, key.borrow()),
        )
    }


    // Immutably borrow the exemplar for the given key, mutually recursing through
    // `Node::get_exemplar`.
    #[inline]
    pub fn get_exemplar(&self, key: &[u8]) -> &Leaf<K, V> {
        self.exemplar(key.borrow()).get_exemplar(key)
    }


    // Mutably borrow the exemplar for the given key, mutually recursing through
    // `Node::get_exemplar_mut`.
    #[inline]
    pub fn get_exemplar_mut(&mut self, key: &[u8]) -> &mut Leaf<K, V> {
        self.exemplar_mut(key.borrow()).get_exemplar_mut(key)
    }


    // Convenience method for inserting a leaf into the branch's sparse array.
    #[inline]
    pub fn insert_leaf(&mut self, leaf: Leaf<K, V>) -> &mut Leaf<K, V> {
        self.entries
            .insert(
                nybble_index(self.choice, leaf.key_slice()),
                Node::Leaf(leaf),
            )
            .unwrap_leaf_mut()
    }


    // Convenience method for inserting a branch into the branch's sparse array.
    #[inline]
    pub fn insert_branch(&mut self, index: u8, branch: Branch<K, V>) -> &mut Branch<K, V> {
        self.entries
            .insert(index, Node::Branch(branch))
            .unwrap_branch_mut()
    }


    // Assuming that the provided index is valid, remove the node with that nybble index and
    // return it.
    #[inline]
    pub fn remove(&mut self, index: u8) -> Node<K, V> {
        self.entries.remove(index)
    }


    // Assuming that the branch node has only one element back, remove it and return it in
    // preparation for replacement with a leaf.
    #[inline]
    pub fn clear_last(&mut self) -> Node<K, V> {
        self.entries.clear_last()
    }
}


impl<K: ToOwned, V> Branch<K, V> {
    // Count the number of entries stored in this branch. This traverses all subnodes of the
    // branch, so it is relatively expensive.
    #[inline]
    pub fn count(&self) -> usize {
        self.entries.iter().map(Node::count).sum()
    }


    #[inline]
    pub fn iter(&self) -> ::std::slice::Iter<Node<K, V>> {
        self.entries.iter()
    }


    #[inline]
    pub fn iter_mut(&mut self) -> ::std::slice::IterMut<Node<K, V>> {
        self.entries.iter_mut()
    }
}


impl<K: ToOwned, V> IntoIterator for Branch<K, V> {
    type IntoIter = ::std::vec::IntoIter<Node<K, V>>;
    type Item = Node<K, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}


// A node in the trie. `K` must be `ToOwned` because the `Owned` version is what we store.
pub enum Node<K: ToOwned, V> {
    Leaf(Leaf<K, V>),
    Branch(Branch<K, V>),
}


impl<K: ToOwned + Borrow<[u8]>, V: PartialEq> PartialEq for Node<K, V> {
    #[inline]
    fn eq(&self, rhs: &Node<K, V>) -> bool {
        match (self, rhs) {
            (&Node::Leaf(ref l), &Node::Leaf(ref r)) => l == r,
            (&Node::Branch(ref l), &Node::Branch(ref r)) => l == r,
            _ => false,
        }
    }
}


impl<K: ToOwned, V: Clone> Clone for Node<K, V> {
    #[inline]
    fn clone(&self) -> Self {
        match *self {
            Node::Leaf(ref leaf) => Node::Leaf(leaf.clone()),
            Node::Branch(ref branch) => Node::Branch(branch.clone()),
        }
    }
}


impl<K: ToOwned, V: fmt::Debug> fmt::Debug for Node<K, V>
where
    K::Owned: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Node::Leaf(ref leaf) => {
                f.debug_struct("Leaf")
                    .field("key", &leaf.key)
                    .field("val", &leaf.val)
                    .finish()
            }
            Node::Branch(ref branch) => {
                f.debug_struct("Branch")
                    .field("choice", &branch.choice)
                    .field("entries", &branch.entries)
                    .finish()
            }
        }
    }
}


impl<K: ToOwned + Borrow<[u8]>, V> Node<K, V> {
    // The following `unwrap_` functions are used for (at times) efficiently circumventing the
    // borrowchecker. All of them use `debug_unreachable!` internally, which means that in release,
    // a misuse can cause undefined behavior (because the tried-to-unwrap-wrong-thing code path is
    // likely to be statically eliminated.)

    #[inline]
    pub fn unwrap_leaf(self) -> Leaf<K, V> {
        match self {
            Node::Leaf(leaf) => leaf,
            Node::Branch(..) => unsafe { debug_unreachable!() },
        }
    }

    #[inline]
    pub fn unwrap_leaf_ref(&self) -> &Leaf<K, V> {
        match *self {
            Node::Leaf(ref leaf) => leaf,
            Node::Branch(..) => unsafe { debug_unreachable!() },
        }
    }

    #[inline]
    pub fn unwrap_leaf_mut(&mut self) -> &mut Leaf<K, V> {
        match *self {
            Node::Leaf(ref mut leaf) => leaf,
            Node::Branch(..) => unsafe { debug_unreachable!() },
        }
    }

    #[inline]
    pub fn unwrap_branch_ref(&self) -> &Branch<K, V> {
        match *self {
            Node::Leaf(..) => unsafe { debug_unreachable!() },
            Node::Branch(ref branch) => branch,
        }
    }


    #[inline]
    pub fn unwrap_branch_mut(&mut self) -> &mut Branch<K, V> {
        match *self {
            Node::Leaf(..) => unsafe { debug_unreachable!() },
            Node::Branch(ref mut branch) => branch,
        }
    }


    // Borrow the associated leaf for a given key, if it exists in the trie.
    pub fn get(&self, key: &[u8]) -> Option<&Leaf<K, V>> {
        match *self {
            Node::Leaf(ref leaf) if leaf.key_slice() == key => Some(leaf),
            Node::Leaf(..) => None,

            Node::Branch(ref branch) => branch.get(key),
        }
    }


    // Mutably borrow the associated leaf for a given key, if it exists in the trie.
    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut Leaf<K, V>> {
        match *self {
            Node::Leaf(ref mut leaf) if leaf.key_slice() == key => Some(leaf),
            Node::Leaf(..) => None,

            Node::Branch(ref mut branch) => branch.get_mut(key),
        }
    }


    // Borrow the "exemplar" for a given key, if it exists. The exemplar is any leaf which exists
    // as a child of the same branch that the given key would be inserted into. This is necessary
    // to decide whether or not a new value for the given key can be inserted into an arbitrary
    // branch in the trie, as otherwise the invariant of branch choice points strictly increasing
    // with depth may be violated.
    //
    // If the key already exists in the trie, then the leaf containing it is returned as the
    // exemplar.
    pub fn get_exemplar(&self, key: &[u8]) -> &Leaf<K, V> {
        match *self {
            Node::Leaf(ref leaf) => leaf,
            Node::Branch(ref branch) => branch.get_exemplar(key),
        }
    }


    // Mutably borrow the exemplar for a given key.
    pub fn get_exemplar_mut(&mut self, key: &[u8]) -> &mut Leaf<K, V> {
        match *self {
            Node::Leaf(ref mut leaf) => leaf,
            Node::Branch(ref mut branch) => branch.get_exemplar_mut(key),
        }
    }


    // Borrow the node which contains all and only entries with keys beginning with
    // `prefix`, assuming there exists at least one such entry.
    //
    // PRECONDITION:
    // - There exists at least one node in the trie with the given prefix.
    pub fn get_prefix_validated<'a>(&'a self, prefix: &[u8]) -> &'a Node<K, V> {
        match *self {
            Node::Leaf(..) => self,
            Node::Branch(ref branch) => {
                if branch.choice >= prefix.len() * 2 {
                    self
                } else {
                    let child_opt = branch.child(prefix);
                    let child = unsafe { child_opt.unchecked_unwrap() };
                    child.get_prefix_validated(prefix)
                }
            }
        }
    }


    // Borrow the node which contains all and only entries with keys beginning with
    // `prefix`.
    pub fn get_prefix<'a>(&'a self, prefix: &[u8]) -> Option<&'a Node<K, V>> {
        match *self {
            Node::Leaf(ref leaf) if leaf.key_slice().starts_with(prefix) => Some(self),
            Node::Branch(ref branch)
                if branch.get_exemplar(prefix).key_slice().starts_with(prefix) => Some(
                self.get_prefix_validated(prefix),
            ),

            _ => None,
        }
    }


    // Mutably borrow the node which contains all and only entries with keys beginning with
    // `prefix`, assuming there exists at least one such entry.
    //
    // PRECONDITION:
    // - There exists at least one node in the trie with the given prefix.
    pub fn get_prefix_validated_mut<'a>(&'a mut self, prefix: &[u8]) -> &'a mut Node<K, V> {
        match *self {
            Node::Leaf(..) => self,
            Node::Branch(..) => {
                if self.unwrap_branch_mut().choice >= prefix.len() * 2 {
                    self
                } else {
                    let child_opt = self.unwrap_branch_mut().child_mut(prefix);
                    let child = unsafe { child_opt.unchecked_unwrap() };

                    child.get_prefix_validated_mut(prefix)
                }
            }
        }
    }


    // Mutably borrow the node which contains all and only entries with keys beginning with
    // `prefix`.
    pub fn get_prefix_mut<'a>(&'a mut self, prefix: &[u8]) -> Option<&'a mut Node<K, V>> {
        match *self {
            Node::Leaf(..) => {
                if self.unwrap_leaf_ref().key_slice().starts_with(prefix) {
                    Some(self)
                } else {
                    None
                }
            }

            Node::Branch(..) => {
                let has_prefix = {
                    let exemplar = self.unwrap_branch_ref().get_exemplar(prefix);

                    exemplar.key_slice().starts_with(prefix)
                };


                if has_prefix {
                    Some(self.get_prefix_validated_mut(prefix))
                } else {
                    None
                }
            }
        }
    }


    // Insert into the trie with a given "graft point" - the first point of nybble mismatch
    // between the key and an "exemplar" key.
    //
    // PRECONDITION:
    // - The key is not already in the trie.
    pub fn insert_with_graft_point(
        &mut self,
        graft: usize,
        graft_nybble: u8,
        key: K,
        val: V,
    ) -> &mut V {
        match *self {
            Node::Branch(ref mut branch) if branch.choice <= graft => {
                let index = branch.index(key.borrow());

                if branch.has_entry(index) {
                    branch.entry_mut(index).insert_with_graft_point(
                        graft,
                        graft_nybble,
                        key,
                        val,
                    )
                } else {
                    &mut branch.insert_leaf(Leaf::new(key, val)).val
                }
            }

            _ => {
                let node = mem::replace(self, Node::Branch(Branch::new(graft)));
                let graft_branch = self.unwrap_branch_mut();

                match node {
                    Node::Leaf(leaf) => {
                        graft_branch.insert_leaf(leaf);
                    }
                    Node::Branch(branch) => {
                        graft_branch.insert_branch(graft_nybble, branch);
                    }
                }

                &mut graft_branch.insert_leaf(Leaf::new(key, val)).val
            }
        }
    }


    // Insert a node into a nonempty trie.
    pub fn insert(&mut self, key: K, val: V) -> Option<V> {
        match *self {
            Node::Leaf(..) => {
                match nybble_mismatch(self.unwrap_leaf_ref().key_slice(), key.borrow()) {
                    None => Some(mem::replace(&mut self.unwrap_leaf_mut().val, val)),
                    Some(mismatch) => {
                        let leaf = mem::replace(self, Node::Branch(Branch::new(mismatch)))
                            .unwrap_leaf();
                        let branch = self.unwrap_branch_mut();

                        branch.insert_leaf(Leaf::new(key, val));
                        branch.insert_leaf(leaf);

                        None
                    }
                }
            }

            Node::Branch(..) => {
                let (mismatch, mismatch_nybble) = {
                    let exemplar = self.get_exemplar_mut(key.borrow());

                    let mismatch_opt = nybble_mismatch(exemplar.key_slice(), key.borrow());

                    match mismatch_opt {
                        Some(mismatch) => (mismatch, nybble_index(mismatch, exemplar.key_slice())),
                        None => return Some(mem::replace(&mut exemplar.val, val)),
                    }
                };

                self.insert_with_graft_point(mismatch, mismatch_nybble, key, val);

                None
            }
        }
    }


    // `remove_validated` assumes that it is being called on a `Node::Branch`.
    //
    // PRECONDITION:
    // - `self` is of the `Node::Branch` variant.
    pub fn remove_validated(&mut self, key: &[u8]) -> Option<Leaf<K, V>> {
        match *self {
            Node::Leaf(..) => unsafe { debug_unreachable!() },
            Node::Branch(..) => {
                let leaf = {
                    let branch = self.unwrap_branch_mut();
                    let index = branch.index(key);

                    match branch.child_mut(key) {
                        // Removing a leaf means waiting for `self` to be available so we can try
                        // to compress. Also we can't remove in this match arm since `branch` is
                        // borrowed.
                        Some(&mut Node::Leaf(ref leaf)) if leaf.key_slice() == key => {}

                        Some(child @ &mut Node::Branch(..)) => return child.remove_validated(key),
                        _ => return None,
                    };

                    branch.remove(index).unwrap_leaf()
                };

                // We removed a leaf. The branch's arity has reduced - we may be able to compress.
                if self.unwrap_branch_mut().is_singleton() {
                    let node = self.unwrap_branch_mut().clear_last();
                    mem::replace(self, node);
                }

                Some(leaf)
            }
        }
    }


    // Remove a node from the trie with the given key and return its value, if it exists.
    pub fn remove(root: &mut Option<Node<K, V>>, key: &[u8]) -> Option<Leaf<K, V>> {
        match *root {
            Some(Node::Leaf(..))
                if unsafe { root.as_ref().unchecked_unwrap() }
                       .unwrap_leaf_ref()
                       .key_slice() == key => {
                Some(unsafe { root.take().unchecked_unwrap() }.unwrap_leaf())
            }

            Some(ref mut node @ Node::Branch(..)) => node.remove_validated(key),

            _ => None,
        }
    }


    // `remove_prefix_validated` assumes that it is being called on a `Node::Branch`, and also
    // that there exists at least one node with the given prefix.
    //
    // PRECONDITION:
    // - There exists a node in the trie with the given prefix.
    // - `self` is of the `Branch` variant.
    pub fn remove_prefix_validated(&mut self, prefix: &[u8]) -> Option<Node<K, V>> {
        match *self {
            Node::Leaf(..) => unsafe { debug_unreachable!() },
            Node::Branch(..) => {
                let prefix_node = {
                    let branch = self.unwrap_branch_mut();
                    let index = branch.index(prefix);

                    match branch.child_mut(prefix) {
                        // Similar borrow logistics to `remove_validated`.
                        Some(&mut Node::Leaf(ref l)) if l.key_slice().starts_with(prefix) => {}
                        Some(&mut Node::Branch(ref child_branch))
                            if child_branch.choice >= prefix.len() * 2 => {}

                        Some(child @ &mut Node::Branch(..)) => {
                            return child.remove_prefix_validated(prefix)
                        }

                        _ => return None,
                    }

                    branch.remove(index)
                };

                if self.unwrap_branch_mut().is_singleton() {
                    let node = self.unwrap_branch_mut().clear_last();
                    mem::replace(self, node);
                }

                Some(prefix_node)
            }
        }
    }


    // Remove the node which holds all and only elements starting with the given prefix and return
    // it, if it exists.
    pub fn remove_prefix(root: &mut Option<Node<K, V>>, prefix: &[u8]) -> Option<Node<K, V>> {
        match *root {
            Some(Node::Leaf(..))
                if unsafe { root.as_ref().unchecked_unwrap() }
                       .unwrap_leaf_ref()
                       .key_slice()
                       .starts_with(prefix) => root.take(),

            Some(Node::Branch(..))
                if unsafe { root.as_ref().unchecked_unwrap() }
                       .unwrap_branch_ref()
                       .get_exemplar(prefix)
                       .key_slice()
                       .starts_with(prefix) => {
                if unsafe { root.as_ref().unchecked_unwrap() }
                    .unwrap_branch_ref()
                    .choice >= prefix.len()
                {
                    root.take()
                } else {
                    unsafe { root.as_mut().unchecked_unwrap() }.remove_prefix_validated(prefix)
                }
            }

            _ => None,
        }
    }
}


impl<K: ToOwned, V> Node<K, V> {
    pub fn count(&self) -> usize {
        match *self {
            Node::Leaf(..) => 1,
            Node::Branch(ref branch) => branch.count(),
        }
    }


    pub fn iter(&self) -> Iter<K, V> {
        Iter::new(self)
    }


    pub fn iter_mut(&mut self) -> IterMut<K, V> {
        IterMut::new(self)
    }
}


impl<K: ToOwned, V> IntoIterator for Node<K, V> {
    type IntoIter = IntoIter<K, V>;
    type Item = (K::Owned, V);

    fn into_iter(self) -> Self::IntoIter {
        IntoIter::new(self)
    }
}
