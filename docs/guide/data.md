---
title: Look at a project's databases
keywords: data, database, db, postgres, postgresql, sqlite, sql, table, rows, browse, connect, connection, env, docker compose, prisma, schema, read-only, mask, inspect, query, editor, csv, export, count, stats, distinct, nulls, diagram, foreign key, erd, agent
shortcuts: go-data
open: data
---
**Data** ({go-data}) sits under the project's badge next to Design and Architecture. It reads the project's files for the databases it uses: `.env` files, Docker Compose services, Prisma and Drizzle configs, Rails' `database.yml` and SQLite files. Each one shows where it was found and whether it's running. Press **Connect** to open it, or **Add a connection** to paste a URL or a SQLite file path.

Pick a table to browse its rows. Type a condition such as `status = failed` to filter, click a column to sort, and click a row to see all of it on the right. A key that points at another table opens that row, and the rows that point back at it are listed below. The copy button at the top copies the row as JSON.

Everything opens **read-only**, and the database itself enforces it, so nothing done here can change data. Columns that look personal or secret, such as emails, phone numbers, tokens and password hashes, stay hidden until you reveal them. Connections that look like production are marked **PROD** in red and listed last. Passwords you paste are kept in the Keychain, never in Runo's files.

**Query** opens an SQL editor that completes the database's tables and columns, aliases included. Press **Run** or Ctrl-Enter (⌘-Enter on a Mac) to see up to 1,000 rows. Only statements that read run: SELECT, WITH, VALUES, TABLE, EXPLAIN and SHOW, one at a time, inside a read-only transaction that's always rolled back, and functions that act outside it (like ending other sessions or reading the server's files) are refused. **Save query** keeps it under Saved queries for that connection, and recent runs are listed below. **Export CSV** writes every row of a query or of the filtered table, up to a million, with hidden columns left out unless you revealed them.

The status bar's **Count exactly** counts the rows your filters keep, instead of the estimate. The columns button opens each column's share of empty values, distinct values and most common values, estimated from Postgres' statistics; **Count** beside one reads it exactly (SQLite columns are counted this way only). On a **PROD** connection, a filter, sort, query, count or export that would read every row of a big table (more than about 100,000 rows) because no index narrows it waits for **Run anyway**.

**Diagram** draws the tables with a line for each foreign key: tables that others point at sit on the left, and tables with no keys either way come last. Click one to browse it.

The sparkle button asks an agent about the open table, and **Ask about this database** on a connection's page asks about all of it. The New session dialog opens with the schema attached. Agents started from Runo can read the project's databases too, through Runo's data tools: read-only like everything here, with hidden columns always masked and heavy reads on production refused. What they ran is listed on the connection's page under **What agents ran**.

× beside a connection disconnects it and keeps it in the list; **Remove from the list** on its page takes it out, with its saved password. If the database restarts or the computer sleeps, Runo reconnects on its own the next time you load rows.

Postgres and SQLite open today. MySQL, Redis and MongoDB are found and listed, and open in a later release.
