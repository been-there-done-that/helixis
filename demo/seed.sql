-- This script seeds the base tenant, runtime_pack, and artifact for the demo!

INSERT INTO tenants (id, name) 
VALUES ('00000000-0000-0000-0000-000000000001', 'Demo Tenant')
ON CONFLICT (id) DO NOTHING;

INSERT INTO runtime_packs (id, language, language_version, sandbox_kind) 
VALUES ('demo-python-pack', 'python', '3.11', 'proc')
ON CONFLICT (id) DO NOTHING;

INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) 
VALUES ('11111111-1111-1111-1111-111111111111', '00000000-0000-0000-0000-000000000001', 'demo-sha256-hash', 'demo-python-pack', 'main.py', 1024)
ON CONFLICT (id) DO NOTHING;
