-- Schéma de « Suivi patient ». Embarqué dans le binaire (sqlx::migrate!) et joué au démarrage.

CREATE TABLE patients (
    id SERIAL PRIMARY KEY,
    nom TEXT NOT NULL,
    prenom TEXT NOT NULL,
    motif TEXT NOT NULL,
    consignes TEXT NOT NULL DEFAULT '',
    prochain_rdv TIMESTAMPTZ,
    token TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE suivis (
    id SERIAL PRIMARY KEY,
    patient_id INTEGER NOT NULL REFERENCES patients(id) ON DELETE CASCADE,
    douleur SMALLINT NOT NULL CHECK (douleur BETWEEN 0 AND 10),
    tension TEXT,
    commentaire TEXT NOT NULL DEFAULT '',
    lu BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX suivis_patient_idx ON suivis (patient_id, created_at DESC);

-- Sessions praticien côté serveur. Seule l'empreinte SHA-256 du jeton est stockée :
-- une copie de la base ne permet pas de reprendre une session.
CREATE TABLE sessions (
    token_hash BYTEA PRIMARY KEY,
    login TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Journal d'accès : qui, quoi, quel patient, quand, depuis quelle IP.
-- Pas de clé étrangère : le journal doit survivre aux données qu'il trace.
CREATE TABLE journal_acces (
    id BIGSERIAL PRIMARY KEY,
    acteur TEXT NOT NULL,
    action TEXT NOT NULL,
    patient_id INTEGER,
    ip TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX journal_acces_date_idx ON journal_acces (created_at DESC);

-- Ajout seul : toute modification ou suppression d'une entrée est refusée par la base elle-même.
CREATE FUNCTION journal_acces_refus() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'journal_acces est en ajout seul : % refusé', TG_OP;
END;
$$;

CREATE TRIGGER journal_acces_ajout_seul
    BEFORE UPDATE OR DELETE ON journal_acces
    FOR EACH ROW EXECUTE FUNCTION journal_acces_refus();

CREATE TRIGGER journal_acces_sans_truncate
    BEFORE TRUNCATE ON journal_acces
    FOR EACH STATEMENT EXECUTE FUNCTION journal_acces_refus();
