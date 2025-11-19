# DataSource Trait Contract

**Purpose**: Defines the contract for data providers used by GridView.

## Trait Definition

```rust
pub trait DataSource {
    type Row;

    /// Total number of rows (logical length).
    fn len(&self) -> usize;

    /// Access a row by index (0-based). Behind the scenes this may fetch a page.
    fn get(&self, index: usize) -> Option<&Self::Row>;

    /// Refresh underlying data (may fetch from network, disk, etc.).
    fn refresh(&mut self, ctx: &AppContext) -> anyhow::Result<()>;
}
```

## Contract Requirements

### type Row

**Preconditions**: None  
**Postconditions**: 
- Associated type representing a single row of data
- Must implement `Clone` or be reference-stable for rendering

**Constraints**: 
- Should be a struct or enum representing one data item
- Framework will hold references to rows during rendering

### len() -> usize

**Preconditions**: None  
**Postconditions**:
- Returns the total logical number of rows
- Must be consistent: `get(i)` should return `Some(_)` for all `i < len()`
- Should return 0 if no data available

**Side Effects**: None (should not trigger data fetching)

**Performance**: Should be O(1) or very fast (cached value)

### get(&self, index: usize) -> Option<&Self::Row>

**Preconditions**: 
- `index` is a valid usize

**Postconditions**:
- Returns `Some(&row)` if `index < len()`
- Returns `None` if `index >= len()`
- Reference remains valid until next `refresh()` call
- May trigger pagination/fetching if row not in cache

**Side Effects**:
- May trigger network requests or disk I/O for pagination
- Should not modify internal state (immutable reference)

**Performance**: 
- For in-memory sources: O(1)
- For paginated sources: May be slower on cache miss

**Error Handling**: Should return `None` on error rather than panicking

### refresh(&mut self, ctx: &AppContext) -> anyhow::Result<()>

**Preconditions**:
- `ctx` is initialized and valid
- Data source is in valid state

**Postconditions**:
- On success: Data is refreshed, `len()` and `get()` reflect new data
- On error: Returns `Err` with descriptive error message
- Previous references from `get()` may become invalid

**Side Effects**:
- Fetches data from source (network, disk, etc.)
- Updates internal cache/state
- May clear previous data

**Error Handling**:
- Should return `anyhow::Error` with context
- Framework will apply retry policies
- Framework will show error to user via `AppMessage`

**Performance**:
- May be slow (network I/O)
- Framework handles this with retry/timeout policies

## Implementation Patterns

### In-Memory DataSource

```rust
struct InMemoryDataSource {
    rows: Vec<MyRow>,
}

impl DataSource for InMemoryDataSource {
    type Row = MyRow;

    fn len(&self) -> usize {
        self.rows.len()
    }

    fn get(&self, index: usize) -> Option<&MyRow> {
        self.rows.get(index)
    }

    fn refresh(&mut self, ctx: &AppContext) -> anyhow::Result<()> {
        // Fetch all data into memory
        self.rows = ctx.clients.my_service.list_items()?;
        Ok(())
    }
}
```

### Paginated DataSource

```rust
struct PaginatedDataSource {
    page_cache: HashMap<usize, Vec<MyRow>>,
    page_size: usize,
    total_rows: usize,
    client: MyServiceClient,
}

impl DataSource for PaginatedDataSource {
    type Row = MyRow;

    fn len(&self) -> usize {
        self.total_rows
    }

    fn get(&self, index: usize) -> Option<&MyRow> {
        let page = index / self.page_size;
        let offset = index % self.page_size;
        self.page_cache.get(&page)?.get(offset)
    }

    fn refresh(&mut self, ctx: &AppContext) -> anyhow::Result<()> {
        // Update total count, may prefetch first page
        self.total_rows = ctx.clients.my_service.count()?;
        // Clear cache or keep recent pages
        Ok(())
    }
}
```

## Testing Contract

Contract tests should verify:
- `len()` returns consistent value
- `get(i)` returns `Some(_)` for all `i < len()`
- `get(i)` returns `None` for all `i >= len()`
- `refresh()` updates data correctly
- `get()` references remain valid until next `refresh()`
- Error handling is graceful (no panics)

