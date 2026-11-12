CREATE TABLE core.product_versions(
 id uuid PRIMARY KEY,code text NOT NULL CHECK(code~'^[A-Z][A-Z0-9_]{0,31}$'),version integer NOT NULL CHECK(version>0),
 effective_from date NOT NULL,effective_until date NOT NULL CHECK(effective_until>effective_from),
 document jsonb NOT NULL,approval_id uuid NOT NULL UNIQUE,maker_person uuid NOT NULL,checker_person uuid NOT NULL,
 published_at timestamptz NOT NULL DEFAULT now(),UNIQUE(code,version),CHECK(maker_person<>checker_person),
 CHECK(document->>'label'='SYNTHETIC'),CHECK(document->>'chartVersion'='synthetic-chart-v1')
);
REVOKE ALL ON core.product_versions FROM bank_core;
GRANT SELECT ON core.product_versions TO bank_core;
CREATE TRIGGER immutable_products BEFORE UPDATE OR DELETE OR TRUNCATE ON core.product_versions FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION core.publish_product(p_document jsonb,p_approval uuid,p_maker uuid,p_checker uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE original core.product_versions; code_value text:=p_document->>'code';
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(code_value,2));
 SELECT * INTO original FROM core.product_versions WHERE id=(p_document->>'id')::uuid;
 IF FOUND THEN
  IF original.document<>p_document OR original.approval_id<>p_approval THEN RAISE EXCEPTION 'Product identity conflict'; END IF;
  RETURN false;
 END IF;
 IF EXISTS(SELECT 1 FROM core.product_versions WHERE code=code_value AND daterange(effective_from,effective_until,'[)') && daterange((p_document->>'effectiveFrom')::date,(p_document->>'effectiveUntil')::date,'[)')) THEN RAISE EXCEPTION 'Overlapping product effective dates'; END IF;
 INSERT INTO core.product_versions(id,code,version,effective_from,effective_until,document,approval_id,maker_person,checker_person)
 VALUES((p_document->>'id')::uuid,code_value,(p_document->>'version')::integer,(p_document->>'effectiveFrom')::date,(p_document->>'effectiveUntil')::date,p_document,p_approval,p_maker,p_checker);
 RETURN true;
END $$;
GRANT EXECUTE ON FUNCTION core.publish_product(jsonb,uuid,uuid,uuid) TO bank_core;
