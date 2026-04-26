//! Object representation in the runtime

// Re-export TypeTagKind as the canonical type tag for the runtime.
// This eliminates the duplicate TypeTagKind enum and uses the single source
// of truth from core-defs.
use pyaot_core_defs::layout;
pub use pyaot_core_defs::TypeTagKind;
pub use pyaot_core_defs::Value;

/// Iterator kind for different container types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IteratorKind {
    List = 0,
    Tuple = 1,
    Dict = 2,
    String = 3,
    Range = 4,
    Set = 5,
    Bytes = 6,
    Enumerate = 7,
    Zip = 8,
    Map = 9,
    Filter = 10,
    Chain = 11,
    ISlice = 12,
    Zip3 = 13,
    ZipN = 14,
}

impl TryFrom<u8> for IteratorKind {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(IteratorKind::List),
            1 => Ok(IteratorKind::Tuple),
            2 => Ok(IteratorKind::Dict),
            3 => Ok(IteratorKind::String),
            4 => Ok(IteratorKind::Range),
            5 => Ok(IteratorKind::Set),
            6 => Ok(IteratorKind::Bytes),
            7 => Ok(IteratorKind::Enumerate),
            8 => Ok(IteratorKind::Zip),
            9 => Ok(IteratorKind::Map),
            10 => Ok(IteratorKind::Filter),
            11 => Ok(IteratorKind::Chain),
            12 => Ok(IteratorKind::ISlice),
            13 => Ok(IteratorKind::Zip3),
            14 => Ok(IteratorKind::ZipN),
            _ => Err(value),
        }
    }
}

/// Object header (all heap objects start with this)
#[repr(C)]
pub struct ObjHeader {
    pub type_tag: TypeTagKind,
    pub marked: bool, // GC mark bit
    pub size: usize,  // Size in bytes
}

// Compile-time assertion: ObjHeader size must match the layout constant
const _: () = assert!(
    std::mem::size_of::<ObjHeader>() == layout::OBJ_HEADER_SIZE,
    "ObjHeader size does not match layout::OBJ_HEADER_SIZE"
);

/// Base object type (all heap objects start with ObjHeader)
#[repr(C)]
pub struct Obj {
    pub header: ObjHeader,
    // Data follows...
}

impl Obj {
    /// Get the type tag of this object
    pub fn type_tag(&self) -> TypeTagKind {
        self.header.type_tag
    }

    /// Check if object is marked (for GC)
    pub fn is_marked(&self) -> bool {
        self.header.marked
    }

    /// Set mark bit
    pub fn set_marked(&mut self, marked: bool) {
        self.header.marked = marked;
    }
}

/// Float object
#[repr(C)]
pub struct FloatObj {
    pub header: ObjHeader,
    pub value: f64,
}

/// String object
#[repr(C)]
pub struct StrObj {
    pub header: ObjHeader,
    pub len: usize,
    pub data: [u8; 0], // Flexible array member
}

/// Bytes object (same layout as StrObj)
#[repr(C)]
pub struct BytesObj {
    pub header: ObjHeader,
    pub len: usize,
    pub data: [u8; 0], // Flexible array member
}

/// Tombstone marker for deleted entries in hash tables (dict and set).
/// Using the alignment of Obj as the marker value because:
/// 1. It matches what std::ptr::dangling_mut::<Obj>() returns
/// 2. It's not a valid heap pointer (addresses don't start that low)
/// 3. It's different from null (which indicates empty slot)
/// 4. Using a const ensures consistency across all modules
///
/// Note: We can't use std::ptr::dangling_mut::<Obj>() directly because
/// it's not a const function, so we compute the same value manually.
pub const TOMBSTONE: pyaot_core_defs::Value = pyaot_core_defs::Value(0b111);

/// List object — `data` is a uniform `[Value]` array; every slot is a
/// properly-tagged `Value` (Int/Bool immediate or heap pointer).
#[repr(C)]
pub struct ListObj {
    pub header: ObjHeader,
    pub len: usize,
    pub capacity: usize,
    pub data: *mut pyaot_core_defs::Value,
}

/// Tuple object (immutable, inline data).
/// `data` is a flexible array of tagged `Value` slots.
#[repr(C)]
pub struct TupleObj {
    pub header: ObjHeader,
    pub len: usize,
    pub data: [Value; 0],
}

/// Dictionary entry (stored in insertion-order dense array)
#[repr(C)]
pub struct DictEntry {
    pub hash: u64,
    pub key: pyaot_core_defs::Value, // Value(0) = deleted/empty entry
    pub value: pyaot_core_defs::Value,
}

/// Dictionary object (compact hash table preserving insertion order)
///
/// Uses CPython 3.6+ compact dict design:
/// - `indices`: hash index table mapping hash slots to entry indices
///   (-1 = empty, -2 = dummy/deleted, >= 0 = index into entries)
/// - `entries`: dense array of DictEntry in insertion order
#[repr(C)]
pub struct DictObj {
    pub header: ObjHeader,
    pub len: usize,              // Number of active (non-deleted) entries
    pub indices: *mut i64,       // Hash index table
    pub indices_capacity: usize, // Size of indices table (power of 2)
    pub entries: *mut DictEntry, // Dense entries array (insertion order)
    pub entries_len: usize,      // Number of entries including deleted
    pub entries_capacity: usize, // Allocated capacity of entries array
}

/// DefaultDict uses the same DictObj layout with factory_tag packed into
/// the high byte of entries_capacity. See defaultdict.rs for packing details.
/// Deque object — double-ended queue with ring buffer
#[repr(C)]
pub struct DequeObj {
    pub header: ObjHeader,
    pub data: *mut pyaot_core_defs::Value,
    pub capacity: usize,
    pub head: usize,
    pub len: usize,
    pub maxlen: i64, // -1 for unbounded
}

/// Set entry (for open-addressing hash table)
#[repr(C)]
pub struct SetEntry {
    pub hash: u64,
    pub elem: pyaot_core_defs::Value, // Value(0) = empty slot, TOMBSTONE = deleted
}

/// Set object (hash table with open addressing, values only)
#[repr(C)]
pub struct SetObj {
    pub header: ObjHeader,
    pub len: usize,      // Number of active entries
    pub capacity: usize, // Total slots in entries array
    pub entries: *mut SetEntry,
}

/// Instance object (user-defined class instance)
/// Fields are stored inline following the header
#[repr(C)]
pub struct InstanceObj {
    pub header: ObjHeader,
    pub vtable: *const u8,  // Pointer to class vtable for virtual dispatch
    pub class_id: u8,       // ID of the class this is an instance of
    pub field_count: usize, // Number of fields
    pub fields: [pyaot_core_defs::Value; 0], // Flexible array of field values (Value-tagged)
}

// Compile-time assertion: vtable field offset must match the layout constant
const _: () = assert!(
    std::mem::offset_of!(InstanceObj, vtable) == layout::INSTANCE_VTABLE_OFFSET as usize,
    "InstanceObj vtable offset does not match layout::INSTANCE_VTABLE_OFFSET"
);

/// Iterator object for first-class iterator protocol
/// Supports iteration over lists, tuples, dicts, strings, and ranges
#[repr(C)]
pub struct IteratorObj {
    pub header: ObjHeader,
    pub kind: u8,         // IteratorKind
    pub exhausted: bool,  // True when iteration complete
    pub reversed: bool,   // True for reversed iteration
    pub source: *mut Obj, // Container reference (null for range)
    pub index: i64,       // Current position
    pub range_stop: i64,  // For range iterator: stop value
    pub range_step: i64,  // For range iterator: step value
}

/// Generator object for generator functions
/// Stores the execution state and local variables across yield points
///
/// §F.7b: locals is a [Value] array walked uniformly by GC via Value::is_ptr().
/// sent_value is also a tagged Value. The per-slot tag side-array and
/// sent_value_tag field are deleted.
#[repr(C)]
pub struct GeneratorObj {
    pub header: ObjHeader,
    pub func_id: u32,                        // Which generator function this is
    pub state: u32, // Current state (0=initial, 1..N=after yield points, u32::MAX=exhausted)
    pub exhausted: bool, // True when generator is exhausted
    pub closing: bool, // True when close() was called (GeneratorExit pending)
    pub num_locals: u32, // Number of local variables stored
    pub sent_value: pyaot_core_defs::Value, // Value sent via send() — tagged Value
    pub locals: [pyaot_core_defs::Value; 0], // Flexible array: local variables (Value-tagged)
}

// Compile-time assertion: func_id field offset must match the layout constant
const _: () = assert!(
    std::mem::offset_of!(GeneratorObj, func_id) == layout::GENERATOR_FUNC_ID_OFFSET as usize,
    "GeneratorObj func_id offset does not match layout::GENERATOR_FUNC_ID_OFFSET"
);

/// Regex match object for re module
/// Stores match result from re.search() or re.match()
#[repr(C)]
pub struct MatchObj {
    pub header: ObjHeader,
    pub matched: bool,      // Whether the match succeeded
    pub start: i64,         // Start position of match
    pub end: i64,           // End position of match
    pub groups: *mut Obj,   // Tuple of group strings (group 0 is full match)
    pub original: *mut Obj, // Original string that was matched
}

/// File mode for open() builtin
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    Read = 0,              // "r"
    Write = 1,             // "w" (truncate)
    Append = 2,            // "a"
    ReadBinary = 3,        // "rb"
    WriteBinary = 4,       // "wb"
    AppendBinary = 5,      // "ab"
    ReadWrite = 6,         // "r+"
    WriteRead = 7,         // "w+"
    AppendRead = 8,        // "a+"
    ReadWriteBinary = 9,   // "r+b" / "rb+"
    WriteReadBinary = 10,  // "w+b" / "wb+"
    AppendReadBinary = 11, // "a+b" / "ab+"
}

impl TryFrom<u8> for FileMode {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(FileMode::Read),
            1 => Ok(FileMode::Write),
            2 => Ok(FileMode::Append),
            3 => Ok(FileMode::ReadBinary),
            4 => Ok(FileMode::WriteBinary),
            5 => Ok(FileMode::AppendBinary),
            6 => Ok(FileMode::ReadWrite),
            7 => Ok(FileMode::WriteRead),
            8 => Ok(FileMode::AppendRead),
            9 => Ok(FileMode::ReadWriteBinary),
            10 => Ok(FileMode::WriteReadBinary),
            11 => Ok(FileMode::AppendReadBinary),
            _ => Err(value),
        }
    }
}

/// Encoding for text-mode file I/O
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileEncoding {
    Utf8 = 0,   // "utf-8" (default)
    Ascii = 1,  // "ascii"
    Latin1 = 2, // "latin-1" / "iso-8859-1"
}

/// File object for file I/O operations
#[repr(C)]
pub struct FileObj {
    pub header: ObjHeader,
    pub handle: *mut std::fs::File, // Boxed File handle (null if closed)
    pub mode: u8,                   // FileMode enum value
    pub closed: bool,               // True when file is closed
    pub binary: bool,               // True for binary mode (rb/wb/ab)
    pub encoding: u8,               // FileEncoding enum value (for text mode)
    pub name: *mut Obj,             // StrObj with filename
}

/// Zip iterator object (iterates over two iterators in parallel)
/// Layout is compatible with IteratorObj's first fields for kind detection
#[repr(C)]
pub struct ZipIterObj {
    pub header: ObjHeader,
    pub kind: u8,        // Always IteratorKind::Zip
    pub exhausted: bool, // True when either iterator is exhausted
    pub _pad: [u8; 6],   // Padding for alignment
    pub iter1: *mut Obj, // First iterator
    pub iter2: *mut Obj, // Second iterator
}

/// Zip3 iterator object (iterates over three iterators in parallel)
#[repr(C)]
pub struct Zip3IterObj {
    pub header: ObjHeader,
    pub kind: u8,        // Always IteratorKind::Zip3
    pub exhausted: bool, // True when any iterator is exhausted
    pub _pad: [u8; 6],   // Padding for alignment
    pub iter1: *mut Obj, // First iterator
    pub iter2: *mut Obj, // Second iterator
    pub iter3: *mut Obj, // Third iterator
}

/// ZipN iterator object (iterates over N iterators in parallel)
#[repr(C)]
pub struct ZipNIterObj {
    pub header: ObjHeader,
    pub kind: u8,        // Always IteratorKind::ZipN
    pub exhausted: bool, // True when any iterator is exhausted
    pub _pad: [u8; 6],   // Padding for alignment
    pub iters: *mut Obj, // List of iterators
    pub count: i64,      // Number of iterators
}

/// Map iterator object - applies function to each element
/// Layout is compatible with IteratorObj's first fields for kind detection
#[repr(C)]
pub struct MapIterObj {
    pub header: ObjHeader,
    pub kind: u8,             // Always IteratorKind::Map
    pub exhausted: bool,      // True when inner iterator is exhausted
    pub capture_count: u8,    // Number of captures (0-4 supported)
    pub elem_unbox_kind: u8,  // After §F.7c BigBang: 0=passthrough, 1=int unbox, 2=bool unbox
    pub result_box_kind: u8,  // After §F.7c BigBang: 0=passthrough, 1=int box, 2=bool box
    pub _pad: [u8; 3],        // Padding for alignment
    pub func_ptr: i64,        // Function pointer (extern "C" fn(*mut Obj) -> *mut Obj)
    pub inner_iter: *mut Obj, // Inner iterator
    pub captures: *mut Obj,   // Captures tuple (null if no captures)
}

/// Filter iterator object - filters elements by predicate
/// Layout is compatible with IteratorObj's first fields for kind detection
#[repr(C)]
pub struct FilterIterObj {
    pub header: ObjHeader,
    pub kind: u8,             // Always IteratorKind::Filter
    pub exhausted: bool,      // True when inner iterator is exhausted
    pub capture_count: u8,    // Number of captures (0-4 supported)
    pub elem_unbox_kind: u8,  // 0=passthrough, 1=int unbox, 2=bool unbox
    pub _pad: [u8; 4],        // Padding for alignment
    pub func_ptr: i64, // Predicate function pointer (extern "C" fn(*mut Obj) -> i64), 0 for None
    pub inner_iter: *mut Obj, // Inner iterator
    pub captures: *mut Obj, // Captures tuple (null if no captures)
}

/// Chain iterator object - chains multiple iterators sequentially
/// Layout is compatible with IteratorObj's first fields for kind detection
#[repr(C)]
pub struct ChainIterObj {
    pub header: ObjHeader,
    pub kind: u8,         // Always IteratorKind::Chain
    pub exhausted: bool,  // True when all iterators are exhausted
    pub _pad: [u8; 6],    // Padding for alignment
    pub iters: *mut Obj,  // ListObj of iterators
    pub current_idx: i64, // Index of current iterator in the list
    pub num_iters: i64,   // Total number of iterators
}

/// ISlice iterator object - slices an iterator (itertools.islice)
/// Layout is compatible with IteratorObj's first fields for kind detection
#[repr(C)]
pub struct ISliceIterObj {
    pub header: ObjHeader,
    pub kind: u8,             // Always IteratorKind::ISlice
    pub exhausted: bool,      // True when slice is exhausted
    pub _pad: [u8; 6],        // Padding for alignment
    pub inner_iter: *mut Obj, // Inner iterator
    pub next_yield: i64,      // Next index to yield
    pub stop: i64,            // Stop index (-1 for no stop)
    pub step: i64,            // Step value
    pub current: i64,         // Current position in inner iterator
}

/// StringBuilder object for efficient string concatenation
/// Used internally when concatenating 3+ strings to avoid O(n²) copying
#[repr(C)]
pub struct StringBuilderObj {
    pub header: ObjHeader,
    pub len: usize,      // Current total length of accumulated strings
    pub capacity: usize, // Current buffer capacity
    pub data: *mut u8,   // Dynamically growing buffer
}

/// struct_time object for time module
/// Represents a time tuple with named fields (like Python's time.struct_time)
#[repr(C)]
pub struct StructTimeObj {
    pub header: ObjHeader,
    pub tm_year: i64,  // Year (e.g., 2026)
    pub tm_mon: i64,   // Month 1-12
    pub tm_mday: i64,  // Day of month 1-31
    pub tm_hour: i64,  // Hour 0-23
    pub tm_min: i64,   // Minute 0-59
    pub tm_sec: i64,   // Second 0-61 (60-61 for leap seconds)
    pub tm_wday: i64,  // Day of week 0-6 (Monday=0)
    pub tm_yday: i64,  // Day of year 1-366
    pub tm_isdst: i64, // DST flag: -1, 0, or 1
}

/// CompletedProcess object for subprocess module
/// Represents the result of subprocess.run()
#[repr(C)]
pub struct CompletedProcessObj {
    pub header: ObjHeader,
    pub args: *mut Obj,   // list[str] - command and arguments
    pub returncode: i64,  // Exit status
    pub stdout: *mut Obj, // Optional[str] - captured stdout (null if not captured)
    pub stderr: *mut Obj, // Optional[str] - captured stderr (null if not captured)
}

/// ParseResult object for urllib.parse module
/// Represents the result of urlparse()
#[repr(C)]
pub struct ParseResultObj {
    pub header: ObjHeader,
    pub scheme: *mut Obj,   // StrObj - URL scheme (e.g., "https")
    pub netloc: *mut Obj,   // StrObj - Network location (e.g., "example.com:8080")
    pub path: *mut Obj,     // StrObj - Path (e.g., "/path/to/resource")
    pub params: *mut Obj,   // StrObj - Parameters (rarely used, before query)
    pub query: *mut Obj,    // StrObj - Query string (e.g., "key=value")
    pub fragment: *mut Obj, // StrObj - Fragment (e.g., "section1")
}

/// HTTPResponse object for urllib.request module
/// Represents the result of urlopen()
#[repr(C)]
pub struct HttpResponseObj {
    pub header: ObjHeader,
    pub status: i64,       // HTTP status code
    pub url: *mut Obj,     // StrObj - Final URL after redirects
    pub headers: *mut Obj, // DictObj[str, str] - Response headers
    pub body: *mut Obj,    // BytesObj - Response body
}

/// Request object for urllib.request module.
///
/// Mirrors the standard CPython `urllib.request.Request` type: it bundles
/// URL + body + headers + method so callers can build a fully-specified
/// request and pass it to `urlopen()`. All pointer fields are heap objects
/// and are traced by the GC.
#[repr(C)]
pub struct RequestObj {
    pub header: ObjHeader,
    pub url: *mut Obj,     // StrObj
    pub data: *mut Obj,    // BytesObj | null
    pub headers: *mut Obj, // DictObj[str, str] | null
    pub method: *mut Obj,  // StrObj | null — "GET" when null/empty
}

use std::cell::UnsafeCell;
use std::sync::OnceLock;

/// Wrapper that allows `OnceLock` to hold an `UnsafeCell<Obj>`.
///
/// # Safety
/// The None singleton is only accessed from the single-threaded AOT runtime.
/// The `UnsafeCell` is never aliased mutably: the only "mutation" that ever
/// happens is through the `*mut Obj` pointer returned by `none_obj()`, and
/// in practice the None object is never mutated after initialization (its
/// `marked` bit is set to `true` permanently so the GC never touches it).
struct NoneHolder(UnsafeCell<Obj>);

// Safety: The runtime is single-threaded; no concurrent access is possible.
unsafe impl Sync for NoneHolder {}

/// None singleton
static NONE_SINGLETON: OnceLock<NoneHolder> = OnceLock::new();

pub fn none_obj() -> *mut Obj {
    let holder = NONE_SINGLETON.get_or_init(|| {
        NoneHolder(UnsafeCell::new(Obj {
            header: ObjHeader {
                type_tag: TypeTagKind::None,
                marked: true, // Never collect None
                size: std::mem::size_of::<Obj>(),
            },
        }))
    });
    holder.0.get()
}

/// `NotImplemented` singleton — same lifetime/locking discipline as the
/// `None` singleton above. Identity-compared at operator-dunder dispatch
/// sites to detect the "I do not handle this operand" return value and
/// fall back to the right operand's reflected dunder per CPython
/// Data Model §3.3.8.
struct NotImplementedHolder(UnsafeCell<Obj>);
unsafe impl Sync for NotImplementedHolder {}
static NOT_IMPLEMENTED_SINGLETON: OnceLock<NotImplementedHolder> = OnceLock::new();

#[no_mangle]
pub extern "C" fn rt_not_implemented_singleton() -> *mut Obj {
    let holder = NOT_IMPLEMENTED_SINGLETON.get_or_init(|| {
        NotImplementedHolder(UnsafeCell::new(Obj {
            header: ObjHeader {
                type_tag: TypeTagKind::NotImplemented,
                marked: true, // Never collect — singleton lives forever
                size: std::mem::size_of::<Obj>(),
            },
        }))
    });
    holder.0.get()
}
