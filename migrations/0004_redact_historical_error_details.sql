CREATE FUNCTION pg_temp.modelport_safe_error(error_text text)
RETURNS text
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT CASE
        WHEN error_text IS NULL THEN NULL
        WHEN lower(error_text) LIKE '%timed out%'
          OR lower(error_text) LIKE '%timeout%'
            THEN 'request failed: timeout [details redacted]'
        WHEN lower(error_text) LIKE '%insufficient_balance%'
          OR lower(error_text) LIKE '%insufficient balance%'
          OR lower(error_text) LIKE '%insufficient account balance%'
          OR lower(error_text) LIKE '%balance not enough%'
          OR error_text LIKE '%余额不足%'
            THEN 'request failed: insufficient balance [details redacted]'
        WHEN lower(error_text) LIKE '%rate limit%'
          OR lower(error_text) LIKE '%rate_limited%'
            THEN 'request failed: rate limit [details redacted]'
        WHEN lower(error_text) LIKE '%tool%'
          OR lower(error_text) LIKE '%function%'
          OR lower(error_text) LIKE '%input_json%'
          OR lower(error_text) LIKE '%schema path%'
            THEN 'request failed: tool protocol error [details redacted]'
        WHEN lower(error_text) LIKE '%authentication%'
          OR lower(error_text) LIKE '%authorization%'
          OR lower(error_text) LIKE '%api key%'
            THEN 'request failed: authentication or authorization [details redacted]'
        WHEN lower(error_text) LIKE '%configuration%'
          OR lower(error_text) LIKE '%missing secret%'
            THEN 'request failed: configuration [details redacted]'
        ELSE 'request failed [details redacted]'
    END
$$;

UPDATE modelport_gateway_requests
SET error_message = pg_temp.modelport_safe_error(error_message)
WHERE error_message IS NOT NULL;

UPDATE modelport_provider_attempts
SET error_message = pg_temp.modelport_safe_error(error_message)
WHERE error_message IS NOT NULL;
