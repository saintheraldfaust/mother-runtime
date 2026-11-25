-- Task acceptance_criteria JSON now supports textual outcomes as well as legacy
-- file constraints. Existing task rows, checkpoints, handoffs and retry responses
-- are unchanged. Mark the format so older runtimes cannot write this database.
PRAGMA user_version=11;
