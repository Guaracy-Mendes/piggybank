-- owner: core (consumer workspace). Official UI increment (schema 77), local synthetic.
-- UI-10 photo goals: the vault owner can attach one bounded PNG or JPEG (at most 32 KiB) to a vault, replace or remove
-- it. Members of a shared vault see it. A photo is presentation metadata only and never changes a balance or the journal.
CREATE TABLE core.consumer_vault_photos(vault_id uuid PRIMARY KEY REFERENCES core.consumer_vaults(id),person_id uuid NOT NULL,
 mime text NOT NULL CHECK(mime IN('image/png','image/jpeg')),content text NOT NULL CHECK(length(content)<=43692),updated_at timestamptz NOT NULL DEFAULT clock_timestamp());
REVOKE ALL ON core.consumer_vault_photos FROM bank_core;
GRANT SELECT ON core.consumer_vault_photos TO bank_core;

CREATE OR REPLACE FUNCTION core.consumer_command(p_actor jsonb,p_person uuid,p_key uuid,p_action text,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE old core.consumer_commands;target uuid:=gen_random_uuid();result jsonb;keys text[];d jsonb;account uuid;total bigint;refs text[];people uuid[];n integer;share bigint;rest bigint;
 self_share bigint;i integer;other uuid;req core.consumer_requests;pe core.payment_executions;card core.consumer_cards;BEGIN
 IF p_action='CARD_PURCHASE' THEN
  -- The purchase itself is unchanged; approved saving rules apply afterwards, once per purchase and rule.
  result:=core.consumer_command_v59(p_actor,p_person,p_key,p_action,p_data);
  IF result->>'state'='POSTED' THEN
   SELECT details INTO d FROM core.consumer_authorizations WHERE id=(p_data->>'authorizationId')::uuid AND person_id=p_person;
   SELECT * INTO card FROM core.consumer_cards WHERE id=(d->>'id')::uuid AND person_id=p_person;
   IF FOUND THEN result:=result||jsonb_build_object('ruleSavings',core.consumer_apply_save_rules(p_person,(result->>'id')::uuid,card.account_id,card.id,d->>'merchant',d->>'category'));END IF;
  END IF;
  RETURN result;
 END IF;
 IF p_action NOT IN('SPLIT_EXPENSE','REQUEST_MARK_PAID','FAVORITE_SET','FAVORITE_REMOVE','SAVE_RULE_SET','SAVE_RULE_DISABLE','VAULT_PHOTO') THEN
  IF p_action='SCHEDULE_REPLACE' THEN RETURN core.consumer_schedule_replace(p_actor,p_person,p_key,p_data);END IF;
  RETURN core.consumer_command_v59(p_actor,p_person,p_key,p_action,p_data);
 END IF;
 PERFORM core.consumer_actor(p_actor,p_person);
 IF p_key IS NULL OR p_key='00000000-0000-0000-0000-000000000000' THEN RAISE EXCEPTION 'Request key required';END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(p_person::text,307));
 SELECT * INTO old FROM core.consumer_commands WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN IF old.person_id<>p_person OR old.action<>p_action OR old.input<>p_data THEN RAISE EXCEPTION 'Command scope conflict';END IF;RETURN old.result;END IF;
 keys:=CASE p_action WHEN 'SPLIT_EXPENSE' THEN ARRAY['accountId','batchId','references','includeSelf','memo'] WHEN 'REQUEST_MARK_PAID' THEN ARRAY['id','paymentId']
  WHEN 'FAVORITE_SET' THEN ARRAY['reference','label'] WHEN 'FAVORITE_REMOVE' THEN ARRAY['reference'] WHEN 'SAVE_RULE_SET' THEN ARRAY['authorizationId'] WHEN 'VAULT_PHOTO' THEN ARRAY['id','photo'] ELSE ARRAY['id'] END;
 IF jsonb_typeof(p_data) IS DISTINCT FROM 'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>ALL(keys))
  OR EXISTS(SELECT 1 FROM unnest(keys) k WHERE NOT p_data?k OR (p_data->k='null'::jsonb AND NOT (p_action='VAULT_PHOTO' AND k='photo'))) THEN RAISE EXCEPTION 'Complete supported command fields required';END IF;
 IF p_action='SPLIT_EXPENSE' THEN
  account:=(p_data->>'accountId')::uuid;PERFORM core.consumer_account(account,p_person);
  IF length(p_data->>'memo')>120 OR jsonb_typeof(p_data->'references')<>'array' OR jsonb_typeof(p_data->'includeSelf')<>'boolean' THEN RAISE EXCEPTION 'Valid split fields required';END IF;
  SELECT -e.delta INTO total FROM core.journal_entries e WHERE e.batch_id=(p_data->>'batchId')::uuid AND e.account_id=account AND e.delta<0;
  IF total IS NULL THEN RAISE EXCEPTION 'An outgoing transaction on your account is required';END IF;
  SELECT array_agg(DISTINCT btrim(t.x)) INTO refs FROM jsonb_array_elements_text(p_data->'references') t(x) WHERE btrim(t.x)<>'';
  IF refs IS NULL OR cardinality(refs)<>jsonb_array_length(p_data->'references') OR cardinality(refs) NOT BETWEEN 1 AND 9 THEN RAISE EXCEPTION 'Between one and nine distinct receiving references required';END IF;
  people:='{}';
  FOR i IN 1..cardinality(refs) LOOP
   SELECT r.person_id INTO other FROM core.receiving_references rr JOIN core.account_relationships r ON r.account_id=rr.account_id AND r.kind='OWNER' AND r.revoked_at IS NULL
    WHERE rr.reference=refs[i] LIMIT 1;
   IF other IS NULL OR other=p_person OR other=ANY(people) THEN RAISE EXCEPTION 'Each reference must belong to a different other customer';END IF;
   people:=people||other;
  END LOOP;
  n:=cardinality(refs)+CASE WHEN (p_data->>'includeSelf')::boolean THEN 1 ELSE 0 END;
  share:=total/n;rest:=total-share*n;
  IF share<=0 THEN RAISE EXCEPTION 'The amount is too small to split';END IF;
  -- With the requester included, the requester keeps the remainder; otherwise the first participants pay one more minor unit.
  self_share:=CASE WHEN (p_data->>'includeSelf')::boolean THEN share+rest ELSE 0 END;
  INSERT INTO core.consumer_splits VALUES(target,p_person,account,(p_data->>'batchId')::uuid,total,(p_data->>'includeSelf')::boolean,cardinality(refs),self_share,p_data->>'memo',clock_timestamp());
  FOR i IN 1..cardinality(refs) LOOP
   INSERT INTO core.consumer_requests(id,person_id,recipient_person,account_id,amount_minor,memo,state,payment_id,created_at,split_id)
    VALUES(gen_random_uuid(),p_person,people[i],account,share+CASE WHEN NOT (p_data->>'includeSelf')::boolean AND i<=rest THEN 1 ELSE 0 END,p_data->>'memo','REQUESTED',NULL,clock_timestamp(),target);
   PERFORM core.consumer_notify(people[i],'Transactions','Split request received',p_data->>'memo','/payments');
  END LOOP;
  result:=jsonb_build_object('id',target,'totalMinor',total::text,'selfShareMinor',self_share::text,'requests',cardinality(refs));
 ELSIF p_action='REQUEST_MARK_PAID' THEN
  SELECT * INTO req FROM core.consumer_requests WHERE id=(p_data->>'id')::uuid AND recipient_person=p_person FOR UPDATE;
  IF NOT FOUND OR req.state<>'REQUESTED' THEN RAISE EXCEPTION 'Open incoming request required';END IF;
  SELECT * INTO pe FROM core.payment_executions WHERE operation_id=(p_data->>'paymentId')::uuid;
  IF NOT FOUND OR pe.payee_account<>req.account_id OR pe.principal<>req.amount_minor
   OR NOT EXISTS(SELECT 1 FROM core.account_relationships WHERE account_id=pe.payer_account AND person_id=p_person AND kind='OWNER' AND revoked_at IS NULL) THEN
   RAISE EXCEPTION 'A settled payment of the exact amount from your account to the requester is required';END IF;
  IF EXISTS(SELECT 1 FROM core.consumer_requests WHERE payment_id=pe.operation_id) THEN RAISE EXCEPTION 'That payment already settles another request';END IF;
  UPDATE core.consumer_requests SET state='PAID',payment_id=pe.operation_id WHERE id=req.id;
  PERFORM core.consumer_notify(req.person_id,'Transactions','Request paid',req.memo,'/payments');
  result:=jsonb_build_object('id',req.id,'state','PAID','paymentId',pe.operation_id,'batchId',pe.batch_id);
 ELSIF p_action='VAULT_PHOTO' THEN
  PERFORM 1 FROM core.consumer_vaults WHERE id=(p_data->>'id')::uuid AND person_id=p_person FOR UPDATE;IF NOT FOUND THEN RAISE EXCEPTION 'Own vault required';END IF;
  IF p_data->'photo'='null'::jsonb THEN
   DELETE FROM core.consumer_vault_photos WHERE vault_id=(p_data->>'id')::uuid;result:=jsonb_build_object('id',p_data->>'id','photo',false);
  ELSE
   IF jsonb_typeof(p_data->'photo')<>'object' OR p_data->'photo'->>'mime' NOT IN('image/png','image/jpeg') OR coalesce(p_data->'photo'->>'content','')!~'^[A-Za-z0-9+/]+={0,2}$'
    OR length(p_data->'photo'->>'content')>43692 OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data->'photo') k WHERE k NOT IN('mime','content')) THEN
    RAISE EXCEPTION 'A PNG or JPEG photo of at most 32 KiB is required';END IF;
   INSERT INTO core.consumer_vault_photos VALUES((p_data->>'id')::uuid,p_person,p_data->'photo'->>'mime',p_data->'photo'->>'content',clock_timestamp())
    ON CONFLICT(vault_id) DO UPDATE SET mime=EXCLUDED.mime,content=EXCLUDED.content,updated_at=EXCLUDED.updated_at;
   result:=jsonb_build_object('id',p_data->>'id','photo',true);
  END IF;
 ELSIF p_action='FAVORITE_SET' THEN
  IF length(btrim(p_data->>'label')) NOT BETWEEN 1 AND 40 THEN RAISE EXCEPTION 'Favourite label of 1 to 40 characters required';END IF;
  IF NOT EXISTS(SELECT 1 FROM core.receiving_references WHERE reference=p_data->>'reference') THEN RAISE EXCEPTION 'Known receiving reference required';END IF;
  IF (SELECT count(*) FROM core.consumer_favorites WHERE person_id=p_person AND reference<>p_data->>'reference')>=20 THEN RAISE EXCEPTION 'At most 20 favourites';END IF;
  INSERT INTO core.consumer_favorites VALUES(p_person,p_data->>'reference',btrim(p_data->>'label'),clock_timestamp())
   ON CONFLICT(person_id,reference) DO UPDATE SET label=EXCLUDED.label;
  result:=jsonb_build_object('reference',p_data->>'reference','saved',true);
 ELSIF p_action='FAVORITE_REMOVE' THEN
  DELETE FROM core.consumer_favorites WHERE person_id=p_person AND reference=p_data->>'reference';
  result:=jsonb_build_object('reference',p_data->>'reference','removed',FOUND);
 ELSIF p_action='SAVE_RULE_SET' THEN
  d:=core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,p_action);
  PERFORM 1 FROM core.consumer_cards WHERE id=(d->>'cardId')::uuid AND person_id=p_person;IF NOT FOUND THEN RAISE EXCEPTION 'Own card required';END IF;
  PERFORM 1 FROM core.consumer_vaults WHERE id=(d->>'vaultId')::uuid AND person_id=p_person;IF NOT FOUND THEN RAISE EXCEPTION 'Own vault required';END IF;
  IF (SELECT count(*) FROM core.consumer_save_rules WHERE person_id=p_person AND enabled)>=10 THEN RAISE EXCEPTION 'At most 10 active saving rules';END IF;
  INSERT INTO core.consumer_save_rules VALUES(target,p_person,(p_data->>'authorizationId')::uuid,(d->>'cardId')::uuid,(d->>'vaultId')::uuid,d->>'matchKind',btrim(d->>'matchValue'),
   core.consumer_minor(d->>'amountMinor'),core.consumer_minor(d->>'weeklyCapMinor'),true,clock_timestamp());
  result:=jsonb_build_object('id',target,'state','ACTIVE');
 ELSE
  UPDATE core.consumer_save_rules SET enabled=false WHERE id=(p_data->>'id')::uuid AND person_id=p_person AND enabled;
  IF NOT FOUND THEN RAISE EXCEPTION 'Active own saving rule required';END IF;
  result:=jsonb_build_object('id',p_data->>'id','state','DISABLED');
 END IF;
 INSERT INTO core.consumer_commands VALUES(target,p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_person,p_action,p_data,result,clock_timestamp());
 RETURN result;
END $$;
GRANT EXECUTE ON FUNCTION core.consumer_command(jsonb,uuid,uuid,text,jsonb) TO bank_core;

CREATE OR REPLACE FUNCTION core.consumer_snapshot(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE d jsonb;BEGIN
 -- Schema-61 composition: the original snapshot validates actor and ownership first.
 d:=core.consumer_snapshot_v60(p_actor,p_person)||jsonb_build_object('progress',core.consumer_progress_at(p_person,clock_timestamp()));
 RETURN d||jsonb_build_object(
  'favorites',coalesce((SELECT jsonb_agg(jsonb_build_object('reference',reference,'label',label) ORDER BY label,reference) FROM core.consumer_favorites WHERE person_id=p_person),'[]'::jsonb),
  'splits',coalesce((SELECT jsonb_agg(jsonb_build_object('id',s.id,'batchId',s.batch_id,'totalMinor',s.total_minor::text,'selfShareMinor',s.self_share_minor::text,
    'participants',s.participants,'memo',s.memo,'createdAt',s.created_at,
    'paid',(SELECT count(*) FROM core.consumer_requests q WHERE q.split_id=s.id AND q.state='PAID'),
    'requests',(SELECT jsonb_agg(jsonb_build_object('id',q.id,'amountMinor',q.amount_minor::text,'state',q.state) ORDER BY q.amount_minor DESC,q.id) FROM core.consumer_requests q WHERE q.split_id=s.id))
    ORDER BY s.created_at DESC) FROM core.consumer_splits s WHERE s.person_id=p_person),'[]'::jsonb),
  'requestLinks',coalesce((SELECT jsonb_object_agg(q.id,jsonb_build_object('splitId',q.split_id,'paymentId',q.payment_id))
    FROM core.consumer_requests q WHERE (q.person_id=p_person OR q.recipient_person=p_person) AND (q.split_id IS NOT NULL OR q.payment_id IS NOT NULL)),'{}'::jsonb),
  'saveRules',coalesce((SELECT jsonb_agg(jsonb_build_object('id',r.id,'cardId',r.card_id,'vaultId',r.vault_id,'matchKind',r.match_kind,'matchValue',r.match_value,
    'amountMinor',r.amount_minor::text,'weeklyCapMinor',r.weekly_cap_minor::text,'enabled',r.enabled,'createdAt',r.created_at,
    'savedThisWeekMinor',(SELECT coalesce(sum(f.amount_minor),0)::text FROM core.consumer_save_rule_facts f WHERE f.rule_id=r.id AND f.created_at>=date_trunc('week',clock_timestamp())),
    'savedLifetimeMinor',(SELECT coalesce(sum(f.amount_minor),0)::text FROM core.consumer_save_rule_facts f WHERE f.rule_id=r.id)) ORDER BY r.created_at DESC)
    FROM core.consumer_save_rules r WHERE r.person_id=p_person),'[]'::jsonb),
  'safeToSpend',core.consumer_safe_to_spend(p_person,clock_timestamp()),
  'vaultPhotos',coalesce((SELECT jsonb_object_agg(p.vault_id,'data:'||p.mime||';base64,'||p.content) FROM core.consumer_vault_photos p
   WHERE p.vault_id IN (SELECT m.vault_id FROM core.consumer_members m WHERE m.person_id=p_person AND m.state='ACTIVE') OR p.person_id=p_person),'{}'::jsonb));
END $$;
GRANT EXECUTE ON FUNCTION core.consumer_snapshot(jsonb,uuid) TO bank_core;
