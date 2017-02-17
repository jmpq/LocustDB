mod packed_strings;
mod integers;
use value::{ValueType, InpVal, InpRecordType};
use std::boxed::Box;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::rc::Rc;
use std::iter;
use std::cmp;
use heapsize::HeapSizeOf;
use self::packed_strings::StringPacker;
use self::integers::IntegerColumn;
use std::i64;

pub struct Batch {
    pub cols: Vec<Column>,
}

pub struct Column {
    name: String,
    data: Box<ColumnData>,
}

impl Column {
    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn iter<'a>(&'a self) -> ColIter<'a> {
        self.data.iter()
    }

    fn new(name: String, data: Box<ColumnData>) -> Column {
        Column { name: name, data: data }
    }
}

impl HeapSizeOf for Column {
    fn heap_size_of_children(&self) -> usize {
        self.name.heap_size_of_children() + self.data.heap_size_of_children()
    }
}

pub trait ColumnData : HeapSizeOf {
    fn iter<'a>(&'a self) -> ColIter<'a>;
}

pub struct ColIter<'a> {
    iter: Box<Iterator<Item=ValueType<'a>> + 'a>
}

impl<'a> Iterator for ColIter<'a> {
    type Item = ValueType<'a>;

    fn next(&mut self) -> Option<ValueType<'a>> {
        self.iter.next()
    }
}

struct NullColumn {
    length: usize,
}

impl NullColumn {
    fn new(length: usize) -> NullColumn {
        NullColumn {
            length: length,
        }
    }
}

impl ColumnData for NullColumn {
    fn iter<'a>(&'a self) -> ColIter<'a> {
        let iter = iter::repeat(ValueType::Null).take(self.length);
        ColIter{iter: Box::new(iter)}
    }
}


struct TimestampColumn {
    values: Vec<u64>
}

impl TimestampColumn {
    fn new(values: Vec<u64>) -> TimestampColumn {
        TimestampColumn {
            values: values
        }
    }
}

impl ColumnData for TimestampColumn {
    fn iter<'a>(&'a self) -> ColIter<'a> {
        let iter = self.values.iter().map(|&t| ValueType::Timestamp(t));
        ColIter{iter: Box::new(iter)}
    }
}

struct StringColumn {
    values: StringPacker,
}

impl StringColumn {
    fn new(values: Vec<Option<Rc<String>>>) -> StringColumn {
        StringColumn {
            values: StringPacker::from_strings(&values),
        }
    }
}

impl ColumnData for StringColumn {
    fn iter<'a>(&'a self) -> ColIter<'a> {
        let iter = self.values.iter().map(|s| ValueType::Str(s));
        ColIter{iter: Box::new(iter)}
    }
}

struct SetColumn {
    values: Vec<Vec<String>>
}

impl SetColumn {
    fn new(values: Vec<Vec<String>>) -> SetColumn {
        SetColumn {
            values: values
        }
    }
}

impl ColumnData for SetColumn {
    fn iter<'a>(&'a self) -> ColIter<'a> {
        let iter = self.values.iter().map(|s| ValueType::Set(Rc::new(s.clone())));
        ColIter{iter: Box::new(iter)}
    }
}

struct MixedColumn {
    values: Vec<InpVal>,
}

impl MixedColumn {
    fn new(mut values: Vec<InpVal>) -> MixedColumn {
        values.shrink_to_fit();
        MixedColumn {
            values: values,
        }
    }
}

impl ColumnData for MixedColumn {
    fn iter(&self) -> ColIter {
        let iter = self.values.iter().map(|val| val.to_val());
        ColIter{iter: Box::new(iter)}
    }
}


impl HeapSizeOf for Batch {
    fn heap_size_of_children(&self) -> usize {
        self.cols.heap_size_of_children()
    }
}

impl HeapSizeOf for NullColumn {
    fn heap_size_of_children(&self) -> usize {
        0
    }
}

impl HeapSizeOf for TimestampColumn {
    fn heap_size_of_children(&self) -> usize {
        self.values.heap_size_of_children()
    }
}

impl HeapSizeOf for StringColumn {
    fn heap_size_of_children(&self) -> usize {
        self.values.heap_size_of_children()
    }
}

impl HeapSizeOf for SetColumn {
    fn heap_size_of_children(&self) -> usize {
        self.values.heap_size_of_children()
    }
}

impl HeapSizeOf for MixedColumn {
    fn heap_size_of_children(&self) -> usize {
        self.values.heap_size_of_children()
    }
}


enum VecType {
    NullVec(usize),
    TimestampVec(Vec<u64>),
    IntegerVec(Vec<i64>, i64, i64),
    StringVec(Vec<Option<Rc<String>>>),
    SetVec(Vec<Vec<String>>),
    MixedVec(Vec<InpVal>),
}

impl VecType {
    fn new_with_value(value: InpVal) -> VecType {
        use self::VecType::*;
        match value {
            InpVal::Null => NullVec(1),
            InpVal::Timestamp(t) => TimestampVec(vec![t]),
            InpVal::Integer(i) => IntegerVec(vec![i], i64::MAX, i64::MIN),
            InpVal::Str(s) => StringVec(vec![Some(s)]),
            InpVal::Set(s) => SetVec(vec![Rc::try_unwrap(s).unwrap()]),
        }
    }

    fn push(&mut self, value: InpVal) -> Option<VecType> {
        match (self, value) {
            (&mut VecType::NullVec(ref mut n), InpVal::Null) => *n += 1,
            (&mut VecType::NullVec(ref n), InpVal::Str(ref s)) => {
                let mut string_vec = iter::repeat(None).take(*n).collect::<Vec<Option<Rc<String>>>>();
                string_vec.push(Some(s.clone()));
                return Some(VecType::StringVec(string_vec))
            },
            (&mut VecType::TimestampVec(ref mut v), InpVal::Timestamp(t)) => v.push(t),
            (&mut VecType::IntegerVec(ref mut v, ref mut min, ref mut max), InpVal::Integer(i)) => {
                *min = cmp::min(i, *min);
                *max = cmp::max(i, *max);
                v.push(i)
            },
            (&mut VecType::StringVec(ref mut v), InpVal::Str(ref s)) => v.push(Some(s.clone())),
            (&mut VecType::StringVec(ref mut v), InpVal::Null) => v.push(None),
            (&mut VecType::SetVec(ref mut v), InpVal::Set(ref s)) => v.push(Rc::try_unwrap(s.clone()).unwrap()),
            (&mut VecType::MixedVec(ref mut v), ref anyval) => v.push(anyval.clone()),
            (slf, anyval) => {
                let mut new_vec = slf.to_mixed();
                new_vec.push(anyval.clone());
                return Some(new_vec)
            },
        }
        None
    }

    fn push_null(&mut self) -> Option<VecType> {
        match self {
            &mut VecType::NullVec(ref mut n) => *n += 1,
            &mut VecType::StringVec(ref mut v) => v.push(None),
            slf => return slf.push(InpVal::Null),
        }
        None
    }

    fn push_int(&mut self, i: i64) -> Option<VecType> {
        match self {
            &mut VecType::IntegerVec(ref mut v, ref mut min, ref mut max) => {
                *min = cmp::min(i, *min);
                *max = cmp::max(i, *max);
                v.push(i)
            },
            slf => return slf.push(InpVal::Integer(i)),
        }
        None
    }

    fn push_str(&mut self, s: &str) -> Option<VecType> {
        match self {
            &mut VecType::StringVec(ref mut v) => v.push(Some(Rc::new(s.to_string()))),
            slf => return slf.push(InpVal::Str(Rc::new(s.to_string()))),
        }
        None
    }

    fn to_mixed(&self) -> VecType {
        match self {
            &VecType::NullVec(ref n)      => VecType::MixedVec(iter::repeat(InpVal::Null).take(*n).collect()),
            &VecType::TimestampVec(ref v) => VecType::MixedVec(v.iter().map(|t| InpVal::Timestamp(*t)).collect()),
            &VecType::IntegerVec(ref v, ..)   => VecType::MixedVec(v.iter().map(|i| InpVal::Integer(*i)).collect()),
            &VecType::StringVec(ref v)    => VecType::MixedVec(v.iter().map(|s| match s {
                &Some(ref string) => InpVal::Str(string.clone()),
                &None => InpVal::Null,
            }).collect()),
            &VecType::SetVec(ref v)       => VecType::MixedVec(v.iter().map(|s| InpVal::Set(Rc::new(s.clone()))).collect()),
            &VecType::MixedVec(_) => panic!("Trying to convert mixed columns to mixed column!"),
        }
    }

    fn to_column_data(self) -> Box<ColumnData> {
        match self {
            VecType::NullVec(n)      => Box::new(NullColumn::new(n)),
            VecType::TimestampVec(v) => Box::new(TimestampColumn::new(v)),
            VecType::IntegerVec(v, min, max) => IntegerColumn::new(v, min, max),
            VecType::StringVec(v)    => Box::new(StringColumn::new(v)),
            VecType::SetVec(v)       => Box::new(SetColumn::new(v)),
            VecType::MixedVec(v)     => Box::new(MixedColumn::new(v)),
        }
    }
}

pub fn columnarize(records: Vec<InpRecordType>) -> Batch {
    let mut field_map: BTreeMap<&str, VecType> = BTreeMap::new();
    for record in records {
        for (name, value) in record {
            let to_insert = match field_map.entry(name) {
                Entry::Vacant(e) => {
                    e.insert(VecType::new_with_value(value));
                    None
                },
                Entry::Occupied(mut e) => {
                    if let Some(new_vec) = e.get_mut().push(value) {
                        let (name, _) = e.remove_entry();
                        Some((name, new_vec))
                    } else {
                        None
                    }
                },
            };

            if let Some((name, vec)) = to_insert {
                field_map.insert(name, vec);
            }
        }
    }

    let mut columns = Vec::new();
    for (name, values) in field_map {
        columns.push(Column::new(name.to_string(), values.to_column_data()));
    }

    Batch { cols: columns }
}

pub fn fused_csvload_columnarize(filename: &str, batch_size: usize) -> Vec<Batch> {
    use std::fs::File;
    use std::io::BufReader;
    use std::io::BufRead;
    use std::rc::Rc;
    use std::boxed::Box;

    let mut batches = Vec::new();
    let mut partial_columns: Vec<VecType> = Vec::new();
    let file = BufReader::new(File::open(filename).unwrap());
    let mut lines_iter = file.lines();
    let first_line = lines_iter.next().unwrap().unwrap();
    let colnames: Vec<String> = first_line.split(",").map(|s| s.to_owned()).collect();

    for (rownumber, line) in lines_iter.enumerate() {
        for (i, val) in line.unwrap().split(",").enumerate() {
            if partial_columns.len() <= i {
                partial_columns.push(VecType::new_with_value(
                    if val == "" { InpVal::Null }
                    else {
                        match val.parse::<i64>() {
                            Ok(int) => InpVal::Integer(int),
                            Err(_) => InpVal::Str(Rc::new(val.to_string())),
                        }
                    }
                ));
            } else {
                if let Some(new_col) = {
                    if val == "" { partial_columns[i].push_null() }
                    else {
                        match val.parse::<i64>() {
                            Ok(int) => partial_columns[i].push_int(int),
                            Err(_) => partial_columns[i].push_str(val),
                        }
                    }
                } {
                    partial_columns[i] = new_col;
                }
            }
        }

        if rownumber % batch_size == batch_size - 1 {
            batches.push(create_batch(partial_columns, &colnames));
            partial_columns = Vec::new();
        }
    }

    if partial_columns.len() > 0 {
        batches.push(create_batch(partial_columns, &colnames));
    }

    batches
}

fn create_batch(cols: Vec<VecType>, colnames: &Vec<String>) -> Batch {
    let mut columns = Vec::new();
    for (i, col) in cols.into_iter().enumerate() {
        columns.push(Column::new(colnames[i].clone(), col.to_column_data()));
    }
    Batch { cols: columns}
}
