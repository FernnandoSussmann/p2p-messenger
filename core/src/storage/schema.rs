use super::migrate::Migration;

/// The ordered, global registry. Slices own the SQL constants.
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    label: "messages",
    sql: crate::messaging::MIGRATION_MESSAGES_V1,
}];

/// Must equal `MIGRATIONS.last().version` (unit-tested).
pub const SCHEMA_VERSION: u32 = 1;
