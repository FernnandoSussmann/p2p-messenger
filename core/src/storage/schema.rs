use super::migrate::Migration;

/// The ordered, global registry. Slices own the SQL constants.
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        label: "messages",
        sql: crate::messaging::MIGRATION_MESSAGES_V1,
    },
    Migration {
        version: 2,
        label: "settings",
        sql: crate::identity::MIGRATION_SETTINGS,
    },
    Migration {
        version: 3,
        label: "contacts",
        sql: crate::contacts::MIGRATION_CONTACTS,
    },
    Migration {
        version: 4,
        label: "legacy_imports",
        sql: super::legacy::MIGRATION_LEGACY_IMPORTS,
    },
    Migration {
        version: 5,
        label: "settings_theme",
        sql: crate::identity::MIGRATION_SETTINGS_THEME,
    },
];

/// Must equal `MIGRATIONS.last().version` (unit-tested).
pub const SCHEMA_VERSION: u32 = 5;
