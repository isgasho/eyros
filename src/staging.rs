use crate::{Point,Value,Location,write_cache::WriteCache};
use failure::{Error};
use random_access_storage::RandomAccess;
use std::collections::HashSet;
use std::rc::Rc;
use std::cell::RefCell;
use desert::{FromBytes,ToBytes,CountBytes};

pub struct StagingIterator<'b,P,V> where P: Point, V: Value {
  inserts: Rc<RefCell<Vec<(P,V)>>>,
  deletes: Rc<RefCell<HashSet<Location>>>,
  bbox: &'b P::Bounds,
  index: u32
}

impl<'b,P,V> StagingIterator<'b,P,V> where P: Point, V: Value {
  pub fn new (inserts: Rc<RefCell<Vec<(P,V)>>>,
  deletes: Rc<RefCell<HashSet<Location>>>, bbox: &'b P::Bounds) -> Self {
    Self { index: 0, bbox, inserts, deletes }
  }
}

impl<'b,P,V> Iterator for StagingIterator<'b,P,V>
where P: Point, V: Value {
  type Item = Result<(P,V,Location),Error>;
  fn next (&mut self) -> Option<Self::Item> {
    let len = iwrap![self.inserts.try_borrow()].len();
    while (self.index as usize) < len {
      let i = self.index;
      self.index += 1;
      if iwrap![self.deletes.try_borrow()].contains(&(0,i)) {
        continue;
      }
      let (point,value) = &iwrap![self.inserts.try_borrow()][i as usize];
      if point.overlaps(self.bbox) {
        return Some(Ok((*point,value.clone(),(0, i))));
      }
    }
    None
  }
}

pub struct Staging<S,P,V>
where S: RandomAccess<Error=Error>, P: Point, V: Value {
  insert_store: WriteCache<S>,
  delete_store: WriteCache<S>,
  pub inserts: Rc<RefCell<Vec<(P,V)>>>,
  pub deletes: Rc<RefCell<Vec<Location>>>,
  pub delete_set: Rc<RefCell<HashSet<Location>>>
}

impl<S,P,V> Staging<S,P,V>
where S: RandomAccess<Error=Error>, P: Point, V: Value {
  pub fn open (istore: S, dstore: S) -> Result<Self,Error> {
    let mut staging = Self {
      insert_store: WriteCache::open(istore)?,
      delete_store: WriteCache::open(dstore)?,
      inserts: Rc::new(RefCell::new(vec![])),
      deletes: Rc::new(RefCell::new(vec![])),
      delete_set: Rc::new(RefCell::new(HashSet::new()))
    };
    staging.load()?;
    Ok(staging)
  }
  fn load (&mut self) -> Result<(),Error> {
    if !self.insert_store.is_empty()? {
      self.inserts.try_borrow_mut()?.clear();
      let len = self.insert_store.len()?;
      let buf = self.insert_store.read(0, len)?;
      let mut offset = 0;
      while offset < len as usize {
        let (size,pv) = <(P,V)>::from_bytes(&buf[offset..])?;
        self.inserts.try_borrow_mut()?.push(pv);
        offset += size;
      }
    }
    if !self.delete_store.is_empty()? {
      self.deletes.try_borrow_mut()?.clear();
      self.delete_set.try_borrow_mut()?.clear();
      let len = self.delete_store.len()?;
      let buf = self.delete_store.read(0, len)?;
      let mut offset = 0;
      while offset < len as usize {
        let (size,loc) = Location::from_bytes(&buf[offset..])?;
        self.deletes.try_borrow_mut()?.push(loc);
        self.delete_set.try_borrow_mut()?.insert(loc);
        offset += size;
      }
    }
    Ok(())
  }
  pub fn clear (&mut self) -> Result<(),Error> {
    self.clear_inserts()?;
    self.clear_deletes()?;
    Ok(())
  }
  pub fn clear_inserts (&mut self) -> Result<(),Error> {
    self.insert_store.truncate(0)?;
    self.inserts.try_borrow_mut()?.clear();
    Ok(())
  }
  pub fn clear_deletes (&mut self) -> Result<(),Error> {
    self.delete_store.truncate(0)?;
    self.deletes.try_borrow_mut()?.clear();
    self.delete_set.try_borrow_mut()?.clear();
    Ok(())
  }
  pub fn delete (&mut self, deletes: &Vec<Location>) -> Result<(),Error> {
    let mut del_set: HashSet<u32> = HashSet::new();
    for delete in deletes {
      if delete.0 == 0 { del_set.insert(delete.1); }
    }
    let mut i = 0;
    self.inserts.try_borrow_mut()?.retain(|_row| {
      let j = i;
      i += 1;
      !del_set.contains(&j)
    });
    Ok(())
  }
  pub fn bytes (&mut self) -> Result<u64,Error> {
    Ok(self.insert_store.len()? + self.delete_store.len()?)
  }
  pub fn len (&mut self) -> Result<usize,Error> {
    Ok(self.inserts.try_borrow()?.len() + self.deletes.try_borrow()?.len())
  }
  pub fn batch (&mut self, inserts: &Vec<(P,V)>, deletes: &Vec<Location>)
  -> Result<(),Error> {
    // todo: calculate the necessary size before allocating
    let mut i_size = 0;
    for insert in inserts.iter() {
      i_size += insert.count_bytes();
    }
    let mut ibuf = vec![0u8;i_size];
    {
      let mut i_offset = 0;
      for insert in inserts.iter() {
        i_offset += insert.write_bytes(&mut ibuf[i_offset..])?;
      }
    }

    let mut d_size = 0;
    for delete in deletes.iter() {
      d_size += delete.count_bytes();
    }
    let mut dbuf = vec![0u8;d_size];
    {
      let mut d_offset = 0;
      for delete in deletes.iter() {
        d_offset += delete.write_bytes(&mut dbuf[d_offset..])?;
      }
    }

    let i_offset = self.insert_store.len()?;
    self.insert_store.write(i_offset,&ibuf)?;
    let d_offset = self.delete_store.len()?;
    self.delete_store.write(d_offset,&dbuf)?;
    self.inserts.try_borrow_mut()?.extend_from_slice(inserts);
    self.deletes.try_borrow_mut()?.extend_from_slice(deletes);
    for delete in deletes {
      self.delete_set.try_borrow_mut()?.insert(*delete);
    }
    Ok(())
  }
  pub fn commit (&mut self) -> Result<(),Error> {
    self.insert_store.sync_all()?;
    self.delete_store.sync_all()?;
    Ok(())
  }
  pub fn query<'b> (&mut self, bbox: &'b P::Bounds)
  -> StagingIterator<'b,P,V> {
    <StagingIterator<'b,P,V>>::new(
      Rc::clone(&self.inserts),
      Rc::clone(&self.delete_set),
      bbox
    )
  }
}
