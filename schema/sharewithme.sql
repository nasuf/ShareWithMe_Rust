create schema if not exists sharewithme;

create table if not exists sharewithme.items (
    id text primary key,
    final_url text not null unique,
    item jsonb not null,
    status text not null,
    created_at timestamptz not null,
    updated_at timestamptz not null
);

create index if not exists items_created_at_idx on sharewithme.items (created_at desc);
create index if not exists items_status_idx on sharewithme.items (status);
create index if not exists items_item_gin_idx on sharewithme.items using gin (item);

alter table sharewithme.items enable row level security;

-- Production should connect with a dedicated login role, not the default
-- postgres owner. Create the role separately with a generated password:
--
--   create role sharewithme_app login password '<generated-password>';
--
-- If the role exists, this block grants the narrow table access and RLS policy
-- required by the backend runtime.
do $$
begin
    if exists (select 1 from pg_roles where rolname = 'sharewithme_app') then
        execute 'grant usage on schema sharewithme to sharewithme_app';
        execute 'grant select, insert, update, delete on sharewithme.items to sharewithme_app';
        execute 'drop policy if exists sharewithme_app_all on sharewithme.items';
        execute 'create policy sharewithme_app_all on sharewithme.items
            for all to sharewithme_app
            using (true)
            with check (true)';
    end if;
end
$$;
