# Performance

Aperio achieves its incredibly low RAM footprint by storing all data directly on the disk instead of keeping it in memory. To ensure high-speed indexing and searches, keep the following core recommendations in mind:

## Storage Drive

Aperio is heavily optimized for SSDs. Because data is fetched directly from the disk, using a mechanical HDD will severely bottleneck your performance due to significantly slower random read times.

## IDs

Aperio stores all data sorted on disk. Because of this architecture, the structure of your item IDs directly impacts your write throughput.

## Sequential IDs

You should always use sequential IDs; non-sequential IDs (like completely random `UUIDv4` strings) are strictly not recommended as they force Aperio to constantly re-sort data, drastically slowing down writing speeds. If you need string-based sequential IDs, formats like `UUIDv7` or `ULID` are excellent choices.

## Numbered IDs

If possible, use numbered IDs (such as `auto-incrementing` integers). Numbered IDs yield even better performance and lower storage utilization than string-based sequential IDs. They require less disk space and allow Aperio to index and sort the data with the absolute minimum amount of overhead.

If using numbered IDs is not possible for your architecture, do not worry. Using alternative sequential strings (like `UUIDv7` or `ULID`) will still be fast enough for high-performance workloads.

## Workaround for Random IDs

If your existing application tightly relies on completely random identifiers like `UUIDv4` or `NanoID` and you cannot change them, you can use a mapping workaround to avoid the performance penalty.

Instead of passing your random string as the primary item `ID` to Aperio, you can generate a sequential placeholder field. For example, `aperio_id` (using an `auto-incrementing` number, `UUIDv7`, or `ULID`) to act as the official identifier for Aperio. You can then store your original random ID inside the item's content body, allowing you to maintain your application's logic without sacrificing Aperio's speed.
