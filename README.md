# URL Shortener

This is a simple url shortener written in Rust using Axum, Diesel, and
Sqlite.

To setup, run the following commands:

```sh
$ sqlite3 db/db.sqlite < schema.sql
$ cargo run
```

## Current Features

- Easy to use: send a post request to `/` with either json or just a
  string, and you'll get a slug back
- Saves the author's ip
- Counts the number of times that any given url has been used

## Production Environments

I'd _highly_ recommend against using this in a production environment as
it would be very easy to break and it is not particularly secure.
