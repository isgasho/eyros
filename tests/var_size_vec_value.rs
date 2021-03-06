extern crate eyros;
extern crate failure;
extern crate random;
extern crate random_access_disk;
extern crate tempfile;

use eyros::{DB,Row};
use failure::Error;
use random_access_disk::RandomAccessDisk;
use random::{Source,default as rand};
use tempfile::Builder as Tmpfile;

use std::cmp::Ordering;

type P = ((f32,f32),(f32,f32),f32);
type V = Vec<u8>;

#[test]
fn var_size_vec_value() -> Result<(),Error> {
  let dir = Tmpfile::new().prefix("eyros").tempdir()?;
  let mut db: DB<_,_,P,V> = DB::open(
    |name: &str| -> Result<RandomAccessDisk,Error> {
      let p = dir.path().join(name);
      Ok(RandomAccessDisk::builder(p)
        .auto_sync(false)
        .build()?)
    }
  )?;
  let mut r = rand().seed([13,12]);
  let size = 40_000;
  let inserts: Vec<Row<P,V>> = (0..size).map(|_| {
    let xmin: f32 = r.read::<f32>()*2.0-1.0;
    let xmax: f32 = xmin + r.read::<f32>().powf(64.0)*(1.0-xmin);
    let ymin: f32 = r.read::<f32>()*2.0-1.0;
    let ymax: f32 = ymin + r.read::<f32>().powf(64.0)*(1.0-ymin);
    let time: f32 = r.read::<f32>()*1000.0;
    let value: V = {
      let size = (r.read::<f64>() * 200.0) as usize;
      (0..size).map(|_| r.read::<u8>()).collect()
    };
    let point = ((xmin,xmax),(ymin,ymax),time);
    Row::Insert(point, value)
  }).collect();
  let n = 4;
  let batches: Vec<Vec<Row<P,V>>> = (0..n).map(|i| {
    inserts[size/n*i..size/n*(i+1)].to_vec()
  }).collect();
  for batch in batches {
    db.batch(&batch)?;
  }

  {
    let bbox = ((-1.0,-1.0,0.0),(1.0,1.0,1000.0));
    let mut results = vec![];
    for result in db.query(&bbox)? {
      let r = result?;
      results.push((r.0,r.1));
    }
    assert_eq!(results.len(), size, "incorrect length for full region");
    let mut expected: Vec<(P,V)> = inserts.iter().map(|r| {
      match r {
        Row::Insert(point,value) => (*point,value.clone()),
        _ => panic!["unexpected row type"]
      }
    }).collect();
    results.sort_unstable_by(cmp);
    expected.sort_unstable_by(cmp);
    assert_eq!(results, expected, "incorrect results for full region");
  }

  {
    let bbox = ((-0.8,0.1,0.0),(0.2,0.5,500.0));
    let mut results = vec![];
    for result in db.query(&bbox)? {
      let r = result?;
      results.push((r.0,r.1));
    }
    let mut expected: Vec<(P,V)> = inserts.iter()
      .map(|r| {
        match r {
          Row::Insert(point,value) => (*point,value.clone()),
          _ => panic!["unexpected row type"]
        }
      })
      .filter(|r| {
        contains_iv((bbox.0).0,(bbox.1).0, (r.0).0)
        && contains_iv((bbox.0).1,(bbox.1).1, (r.0).1)
        && contains_pt((bbox.0).2,(bbox.1).2, (r.0).2)
      })
      .collect();
    results.sort_unstable_by(cmp);
    expected.sort_unstable_by(cmp);
    assert_eq!(results.len(), expected.len(),
      "incorrect length for partial region");
    assert_eq!(results, expected, "incorrect results for partial region");
  }
  Ok(())
}

fn cmp<T> (a: &T, b: &T) -> Ordering where T: PartialOrd {
  match a.partial_cmp(b) {
    Some(o) => o,
    None => panic!["comparison failed"]
  }
}

fn contains_iv<T> (min: T, max: T, iv: (T,T)) -> bool where T: PartialOrd {
  min <= iv.1 && iv.0 <= max
}
fn contains_pt<T> (min: T, max: T, pt: T) -> bool where T: PartialOrd {
  min <= pt && pt <= max
}
