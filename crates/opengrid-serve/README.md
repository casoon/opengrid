# opengrid-serve

The `opengrid-server` command: reads a database's schema, writes a
configuration proposal, and serves what the edited file names.

```sh
OPENGRID_DATABASE_URL=postgres://… opengrid-server propose   # writes opengrid.json
# read it, edit it, delete "proposal": true
OPENGRID_TOKEN=… OPENGRID_DATABASE_URL=postgres://… opengrid-server --config opengrid.json
```

Guide: <https://casoon.github.io/opengrid/docs/guides/from-a-connection-string/>

Part of [opengrid](https://github.com/casoon/opengrid), a portable Rust data and
query engine with an accessible data grid on top. The crates and the npm package
are released together, so versions move together: use the same version for every
`opengrid-*` crate you depend on.

- Repository: <https://github.com/casoon/opengrid>
- Documentation: <https://casoon.github.io/opengrid/>
- License: MIT OR Apache-2.0
