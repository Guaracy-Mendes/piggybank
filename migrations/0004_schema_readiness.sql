-- owner: platform. Expose only migration watermark, not privileged migration metadata.
CREATE VIEW platform.schema_version AS SELECT COALESCE(max(version),0)::bigint AS current_version FROM public._sqlx_migrations WHERE success;
REVOKE INSERT,UPDATE,DELETE ON platform.schema_version FROM bank_platform;
