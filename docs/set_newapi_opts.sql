INSERT INTO options(key,value) VALUES ('QuotaForNewUser','500000') ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value;
INSERT INTO options(key,value) VALUES ('BaseURL','https://ai.lain42.top') ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value;
INSERT INTO options(key,value) VALUES ('RegisterEnabled','true') ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value;
INSERT INTO options(key,value) VALUES ('SelfUseModeEnabled','false') ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value;
