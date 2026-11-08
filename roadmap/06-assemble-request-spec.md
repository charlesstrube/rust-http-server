# Étape 6 : spécifications détaillées

Ce document découpe `Assembler::on_frame` en petites fonctions. Chaque fonction a un contrat : ce qu'elle reçoit, ce qu'elle renvoie, et ses règles **dans l'ordre où il faut les vérifier**. Les tests de `src/http2/assembler.rs` sont rattachés à chaque règle.

Notation : `Err(X)` veut dire `Err(AssembleError::X)`. « Ok(None) » veut dire « frame consommée, rien de complet ».

[← Étape 6](06-assemble-request.md)

---

## Vue d'ensemble

```
                     on_frame(frame)
                           │
          ┌────────────────┴────────────────┐
          │ un bloc de headers est en cours ?│
          └────────────────┬────────────────┘
              oui │                   │ non
                  ▼                   ▼
          on_continuation      selon le type de frame
                  │            ├─ HEADERS      → on_headers
                  │            ├─ DATA         → on_data
                  │            ├─ CONTINUATION → Err(UnexpectedContinuation)
                  │            └─ autre        → Ok(None)
                  ▼
     END_HEADERS ? ──oui──▶ finish_block ──▶ (END_STREAM ?) ──oui──▶ complete
                                             │
                                             └─non──▶ range le stream dans `streams`

     on_data ──▶ ajoute au body ──▶ (END_STREAM ?) ──oui──▶ complete
```

Fonctions à écrire (toutes privées sauf `on_frame`) :

| Fonction | Rôle |
|---|---|
| `on_frame` | aiguillage, spec 1 |
| `strip_padding` | retire Pad Length et padding, spec 2 |
| `on_headers` | frame HEADERS, spec 3 |
| `on_continuation` | frame CONTINUATION, spec 4 |
| `finish_block` | décode puis valide un bloc complet, spec 5 |
| `build_request` | pseudo-headers → `Request`, spec 6 |
| `on_data` | frame DATA, spec 7 |
| `complete` | END_STREAM : vérifie et rend la requête, spec 8 |

---

## État de l'`Assembler`

Ta structure actuelle est bonne :

| Champ | Contenu | Invariant |
|---|---|---|
| `decoder` | le `Decoder` HPACK, un seul par connexion | voit **tous** les blocs, dans l'ordre, même ceux qu'on rejette |
| `pending` | `Option<PendingBlock>` : le bloc de headers en cours | `Some` uniquement entre un HEADERS sans `END_HEADERS` et la CONTINUATION qui porte `END_HEADERS` |
| `streams` | `HashMap<u32, Stream>` : les requêtes dont les headers sont complets mais pas le body | un stream y entre quand son bloc est fini sans `END_STREAM`, et en sort à `END_STREAM` |
| `last_stream_id` | le plus grand id de HEADERS accepté | ne fait que monter |

`Stream` contient la `Request` (body vide), le body accumulé et le `content-length` annoncé. `PendingBlock` contient le stream id, le `end_stream` retenu et les fragments concaténés.

⚠️ Pour vérifier un flag, utilise **toujours** `flags.contains(Flags::X)`, jamais `flags == Flags::X` : une frame porte souvent plusieurs flags à la fois (`END_HEADERS | END_STREAM` vaut `0x05`, ce qui n'est égal ni à `0x01` ni à `0x04`).

---

## Spec 1 : `on_frame(frame)`

**Entrée** : n'importe quelle frame, y compris PING, SETTINGS, etc.
**Sortie** : `Result<Option<(u32, Request)>, AssembleError>`.

Règles, dans l'ordre :

1. **Si `pending` est `Some`** (un bloc est en cours), alors la frame **doit** être sa suite : on délègue à `on_continuation` (spec 4), quel que soit le type de la frame. C'est `on_continuation` qui rejette ce qui n'est pas une CONTINUATION.
2. Sinon, selon le type :
   - `Headers` → `on_headers` (spec 3) ;
   - `Data` → `on_data` (spec 7) ;
   - `Continuation` → `Err(UnexpectedContinuation)`, puisqu'aucun bloc n'est ouvert ;
   - tout autre type → `Ok(None)`, sans aucune vérification.

⚠️ Pas de contrôle de stream id ici : un PING ou un SETTINGS a le stream id 0, et c'est normal. Les contrôles d'id sont **propres à chaque type**, dans les specs 3 et 7.

Tests : `connection_frames_are_ignored`, `continuation_without_an_open_block_is_an_error`, `frame_inside_a_header_block_is_an_error`.

---

## Spec 2 : `strip_padding(payload, flags)`

**Entrée** : le payload brut d'une frame HEADERS ou DATA, et ses flags.
**Sortie** : `Result<&[u8], AssembleError>` : la tranche **utile** du payload (fragment de bloc HPACK pour HEADERS, données pour DATA).

Règles, dans l'ordre :

1. Si `PADDED` est présent :
   - le payload doit contenir au moins 1 octet, sinon `Err(InvalidPadding)` ;
   - `pad_len` = premier octet ; `rest` = tout ce qui suit.
   Sinon : `pad_len = 0` et `rest = payload`.
2. Si `PRIORITY` est présent (seulement pour HEADERS, DATA n'a jamais ce flag) :
   - `rest` doit contenir au moins 5 octets, sinon `Err(InvalidPadding)`. La vraie erreur est un `FRAME_SIZE_ERROR`, l'étape 8 corrigera, ce cas n'est pas testé ;
   - on saute ces 5 octets : `rest = rest[5..]`. Leur contenu est ignoré.
3. Si `pad_len > rest.len()` → `Err(InvalidPadding)`.
4. Résultat : `rest` privé de ses `pad_len` derniers octets.

Remarque : `pad_len == rest.len()` est **permis**, ça donne une tranche vide.

Exemples :

| Flags | Payload | Résultat |
|---|---|---|
| aucun | `82 86 84` | `82 86 84` |
| `PADDED` | `02 68 69 00 00` | `68 69` (« hi ») |
| `PRIORITY` | `80 00 00 00 0f 82 86` | `82 86` |
| `PADDED \| PRIORITY` | `02 00 00 00 00 0f 82 86 00 00` | `82 86` |
| `PADDED` | `0a 82 86` | `Err(InvalidPadding)` : 10 > 2 |
| `PADDED` | `05 68` | `Err(InvalidPadding)` : 5 > 1 |

Tests : `padded_headers_drop_the_padding`, `priority_fields_are_skipped`, `padded_and_priority_together`, `padding_longer_than_the_payload_is_an_error`, `padded_data_drops_the_padding`, `data_padding_longer_than_the_payload_is_an_error`.

Astuce Rust : `slice.split_first()` renvoie `Option<(&u8, &[u8])>`, pratique pour l'étape 1. `rest.get(5..)` renvoie `None` au lieu de paniquer si la tranche est trop courte.

---

## Spec 3 : `on_headers(frame)`

Appelée seulement quand aucun bloc n'est en cours.

Règles, dans l'ordre :

1. **Stream id** (RFC 9113 §5.1.1) : si `id == 0`, ou `id` est pair, ou `id <= last_stream_id` → `Err(InvalidStreamId)`.
   Ce seul test couvre : id 0, id pair, id qui redescend (3 puis 1), id réutilisé (1 puis 1).
2. `last_stream_id = id`. On le fait **maintenant**, avant de décoder : même si la requête s'avère malformée, cet id est consommé.
3. `fragment = strip_padding(payload, flags)?` (spec 2).
4. Si `fragment.len() > MAX_HEADER_BLOCK_SIZE` → `Err(HeaderBlockTooLarge)`.
5. Crée un `PendingBlock { stream_id: id, end_stream: flags.contains(END_STREAM), block: fragment.to_vec() }`.
6. Si `END_HEADERS` est présent → `finish_block(pending)` (spec 5) et renvoie son résultat.
   Sinon → `self.pending = Some(pending)`, renvoie `Ok(None)`.

Tests : `even_stream_id_is_an_error`, `stream_id_zero_is_an_error`, `decreasing_stream_id_is_an_error`, `reused_stream_id_is_an_error`, `stream_ids_may_skip`, `headers_without_end_stream_wait_for_data`, plus tous ceux qui passent par `finish_block`.

---

## Spec 4 : `on_continuation(frame)`

Appelée seulement quand `pending` est `Some`.

Règles, dans l'ordre :

1. Sors le bloc : `let pending = self.pending.take()`. Ton code fait déjà ça, c'est le bon réflexe d'ownership.
2. Si la frame n'est **pas** une CONTINUATION → `Err(ExpectedContinuation)`.
3. Si `frame.stream_id() != pending.stream_id` → `Err(ExpectedContinuation)`.
   ⚠️ Compare avec `pending.stream_id`, **pas** avec `last_stream_id`. Le résultat est le même ici, mais c'est `pending` qui porte l'information. Et l'erreur attendue est `ExpectedContinuation`, pas `InvalidStreamId`.
4. Si `pending.block.len() + payload.len() > MAX_HEADER_BLOCK_SIZE` → `Err(HeaderBlockTooLarge)`. On vérifie **avant** d'ajouter, pour ne jamais allouer au-delà de la limite.
5. Ajoute le payload au bloc. Une CONTINUATION n'a ni padding ni priority : on prend le payload tel quel.
6. Si `END_HEADERS` est présent → `finish_block(pending)`.
   Sinon → remets-le : `self.pending = Some(pending)`, renvoie `Ok(None)`.

Note : `END_STREAM` ne se lit **jamais** sur une CONTINUATION. Il a été retenu dans `pending.end_stream` au moment du HEADERS.

Tests : `header_block_split_over_continuation`, `header_block_split_over_two_continuations`, `frame_inside_a_header_block_is_an_error`, `continuation_on_another_stream_is_an_error`, `header_block_larger_than_the_limit_is_an_error`.

Le test de la limite envoie un HEADERS de **exactement** 16 384 octets (accepté, car pas `>`), puis une CONTINUATION de 1 octet : 16 385 > 16 384 → erreur.

---

## Spec 5 : `finish_block(pending)`

Appelée quand le bloc est complet (`END_HEADERS` reçu).

Règles, dans l'ordre :

1. **Décode d'abord, toujours** : `fields = self.decoder.decode(&pending.block)`. En cas d'erreur → `Err(Hpack(e))`.
   C'est la règle la plus importante de l'étape. Si tu valides avant de décoder et que tu rejettes la requête sans la décoder, la table dynamique HPACK ne reçoit pas les entrées que le client y a ajoutées, et **tous** les blocs suivants seront mal lus.
2. `build_request(fields)` (spec 6). En cas d'erreur → `Err(MalformedRequest)`. Le stream n'est rangé nulle part : il est considéré comme fermé.
3. Si `pending.end_stream` est vrai → la requête n'a pas de body : `complete(id, stream avec body vide)` (spec 8).
4. Sinon → range un `Stream { request, body: vide, content_length }` dans `streams` sous l'id, renvoie `Ok(None)`.

Tests : `get_in_a_single_headers_frame`, `rfc_7541_c_4_1_request`, `undecodable_block_is_an_hpack_error`, `malformed_request_still_updates_the_dynamic_table`.

---

## Spec 6 : `build_request(fields)`

**Entrée** : la liste décodée, des `(Vec<u8>, Vec<u8>)` dans l'ordre du fil.
**Sortie** : `Result<(Request, Option<usize>), …>`, avec le `content-length` s'il y en a un. Côté erreur, n'importe quoi qui devient `MalformedRequest` dans `finish_block`.

**Variables de travail** : `method`, `scheme`, `path`, `authority`, toutes `Option<String>` à `None` ; `seen_regular = false` ; `content_length = None` ; `headers = Headers::default()`.

**Pour chaque `(name, value)`, dans l'ordre :**

1. Convertis `name` et `value` en `&str` (`std::str::from_utf8`). Si ce n'est pas de l'UTF-8 → malformé.
2. Si `name` contient une majuscule ASCII → malformé (§8.2.1). Utilise `u8::is_ascii_uppercase`.
3. **Si `name` commence par `:`** (pseudo-header) :
   - si `seen_regular` est vrai → malformé : pseudo-header après un header normal ;
   - si `name` vaut `:method`, `:scheme`, `:path` ou `:authority` : si la variable correspondante est déjà `Some` → malformé (doublon), sinon stocke la valeur ;
   - tout autre nom (`:status`, `:foo`…) → malformé.
4. **Sinon** (header normal) :
   - `seen_regular = true` ;
   - si `name` vaut `connection`, `keep-alive`, `proxy-connection`, `transfer-encoding` ou `upgrade` → malformé (§8.2.2) ;
   - si `name == "te"` et `value != "trailers"` → malformé ;
   - si `name == "content-length"` : parse en `usize`. Si ça échoue → malformé, sinon garde-le dans `content_length` ;
   - ajoute `(name, value)` à `headers`. `content-length` aussi, c'est un header comme un autre pour les handlers.

**Après la boucle :**

5. `method`, `scheme` et `path` doivent tous être `Some`, sinon malformé.
6. `path` vide → malformé.
7. Si `authority` est `Some` → ajoute-la dans `headers` sous le nom `host`.
8. `Request::new(path, method, headers, …)` avec un body vide. Si ça échoue (méthode inconnue comme `DELETE`) → malformé.
9. Renvoie `(request, content_length)`.

`scheme` n'est pas utilisé ensuite : on vérifie seulement sa présence.

Tests : `authority_becomes_host`, `pseudo_headers_are_not_regular_headers`, `authority_is_optional`, `mandatory_pseudo_headers_are_required`, `empty_path_is_malformed`, `duplicated_pseudo_header_is_malformed`, `unknown_or_response_pseudo_header_is_malformed`, `pseudo_header_after_a_regular_header_is_malformed`, `uppercase_header_name_is_malformed`, `connection_specific_headers_are_malformed`, `te_is_only_allowed_as_trailers`.

Astuce Rust : `matches!(name, "connection" | "keep-alive" | "proxy-connection" | "transfer-encoding" | "upgrade")` tient en une ligne. Un `match name { ":method" => …, ":scheme" => …, _ => … }` évite les `if` en cascade.

---

## Spec 7 : `on_data(frame)`

Appelée seulement quand aucun bloc n'est en cours.

Règles, dans l'ordre :

1. Si `id == 0` → `Err(InvalidStreamId)`.
2. Cherche le stream dans `streams` :
   - **absent et `id > last_stream_id`** : le stream n'a jamais été ouvert (idle) → `Err(InvalidStreamId)` ;
   - **absent et `id <= last_stream_id`** : le stream a existé et est terminé (END_STREAM déjà reçu, ou rejeté comme malformé) → `Err(StreamClosed)`.
3. `data = strip_padding(payload, flags)?` (spec 2).
4. Ajoute `data` au `body` du stream.
5. Si `END_STREAM` est présent → **retire** le stream de la map (`HashMap::remove` te le rend par valeur) et renvoie `complete(id, stream)`.
   Sinon → `Ok(None)`.

⚠️ Ne touche pas à `last_stream_id` dans `on_data` : seul un HEADERS ouvre un stream.

Tests : `body_spread_over_two_data_frames`, `empty_data_frame_can_end_the_stream`, `body_is_binary`, `data_of_two_streams_can_interleave`, `data_on_an_idle_stream_is_an_error`, `data_on_stream_zero_is_an_error`, `data_after_end_stream_is_stream_closed`.

---

## Spec 8 : `complete(id, stream)`

Appelée à `END_STREAM`, que ce soit depuis `finish_block` (pas de body) ou depuis `on_data`.

Règles, dans l'ordre :

1. Si `stream.content_length` vaut `Some(n)` et que `n != stream.body.len()` → `Err(MalformedRequest)` (§8.1.1, anti request smuggling).
2. Place le body dans la requête : `request.set_body(body)`.
3. Renvoie `Ok(Some((id, request)))`.

⚠️ `Request::set_body` prend encore une `String` dans ton code. Il doit prendre un `Vec<u8>`, sinon `body_is_binary` (octets `ff 00 fe`, pas de l'UTF-8) ne peut pas passer. Pense à adapter `http1` qui l'appelle.

Tests : `content_length_mismatch_is_malformed`, `matching_content_length_is_accepted`.

---

## Ce que ton code actuel fait de travers

Ton squelette (structures, `pending.take()`) est bien parti. Cinq points font échouer des tests :

1. **Le contrôle « id pair » est en tête de `on_frame`, pour toutes les frames.** Un PING a l'id 0, qui est pair, donc il est rejeté. Déplace ce contrôle dans `on_headers` (spec 3) et `on_data` (spec 7).
2. **`frame.flags() == Flags::END_STREAM`** compare tous les bits d'un coup : `0x05 != 0x01`. Utilise `contains`.
3. **Dans le bloc `pending`, le stream id est comparé à `last_stream_id`** et l'erreur renvoyée est `InvalidStreamId`. Il faut comparer à `pending.stream_id` et renvoyer `ExpectedContinuation` (spec 4).
4. **Après le traitement de `pending`, le `match` sur le type s'exécute quand même** : la CONTINUATION retombe dans `_ => Ok(None)`. Le cas « bloc en cours » doit **se terminer** dans `on_continuation`, avec un `return`.
5. **`on_data` modifie `last_stream_id`** : seul HEADERS doit le faire.

---

## Ordre de travail conseillé

| # | À faire | Tests qui passent au vert |
|---|---|---|
| 1 | Spec 1 (aiguillage), en corrigeant les points 1 et 4 ci-dessus | `connection_frames_are_ignored` |
| 2 | Spec 3 étapes 1-2, spec 5, spec 6 en version minimale (juste `:method`, `:path`, `:authority` → `host`), spec 8 sans `content-length` | `get_in_a_single_headers_frame`, `rfc_7541_c_4_1_request`, `authority_becomes_host`, `pseudo_headers_are_not_regular_headers`, `undecodable_block_is_an_hpack_error`, toute la section *Stream ids* |
| 3 | Spec 2 (padding et priority) | `padded_headers_*`, `priority_*`, `padded_and_priority_together`, `padding_longer_*` |
| 4 | Spec 4 (CONTINUATION) | `header_block_split_*`, `*continuation*`, `frame_inside_a_header_block_is_an_error`, `header_block_larger_than_the_limit_is_an_error` |
| 5 | `set_body(Vec<u8>)`, puis spec 7 | section *Body*, section *Stream states* |
| 6 | Spec 6 complète, plus `content-length` dans spec 8 | section *Malformed requests*, `content_length_*`, `malformed_request_still_updates_the_dynamic_table` |

Lance seulement la partie qui t'intéresse avec un filtre, par exemple :

```sh
cargo test http2::assembler::test::padd
```
