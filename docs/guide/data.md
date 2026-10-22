---
title: Look at a project's databases
keywords: data, database, db, postgres, postgresql, sqlite, sql, table, rows, browse, connect, connection, env, docker compose, prisma, schema, read-only, mask, inspect
shortcuts: go-data
open: data
---
**Data** ({go-data}) sits under the project's badge next to Design and Architecture. It reads the project's files for the databases it uses: `.env` files, Docker Compose services, Prisma and Drizzle configs, Rails' `database.yml` and SQLite files. Each one shows where it was found and whether it's running. Press **Connect** to open it, or **Add a connection** to paste a URL or a SQLite file path.

Pick a table to browse its rows. Type a condition such as `status = failed` to filter, click a column to sort, and click a row to see all of it on the right. A key that points at another table opens that row, and the rows that point back at it are listed below. The copy button at the top copies the row as JSON.

Everything opens **read-only**, and the database itself enforces it, so nothing done here can change data. Columns that look personal or secret, such as emails, phone numbers, tokens and password hashes, stay hidden until you reveal them. Connections that look like production are marked **PROD** in red and listed last. Passwords you paste are kept in the Keychain, never in Runo's files.

× beside a connection disconnects it and keeps it in the list; **Remove from the list** on its page takes it out, with its saved password. If the database restarts or the computer sleeps, Runo reconnects on its own the next time you load rows.

Postgres and SQLite open today. MySQL, Redis and MongoDB are found and listed, and open in a later release.
