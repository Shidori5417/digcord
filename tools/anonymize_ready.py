"""Kaydedilmiş READY'den küçük, anonim bir test fixture'ı üretir.

Kullanım: python3 -I tools/anonymize_ready.py captures/gateway-XXXX.json > fixture.json
Sadece ayrıştırıcının kullandığı alanlar + bilinmeyen alan testi için birkaç zararsız alan kalır.
Tüm ID'ler tutarlı biçimde sahte snowflake'lere, tüm isimler sıradan adlara çevrilir.
"""
import json
import sys

src = json.load(open(sys.argv[1]))
ready = next(m for m in src["recv"] if m.get("t") == "READY")["d"]

ids = {}
def fid(real):
    if real is None:
        return None
    if real not in ids:
        ids[real] = str(100000000000000000 + len(ids) * 4194304)
    return ids[real]

def channel(c, n):
    out = {"id": fid(c["id"]), "type": c["type"], "flags": 0}
    if c.get("parent_id"):
        out["parent_id"] = fid(c["parent_id"])
    if "position" in c:
        out["position"] = c["position"]
    if c.get("name") is not None:
        out["name"] = ("kategori-" if c["type"] == 4 else "kanal-") + str(n)
    return out

guilds = []
for gi, g in enumerate(ready["guilds"][:2], 1):
    chans = g.get("channels", [])
    cats = [c for c in chans if c["type"] == 4][:1]
    rest = [c for c in chans if c["type"] != 4 and (not cats or c.get("parent_id") == cats[0]["id"])][:2]
    guilds.append({
        "id": fid(g["id"]),
        "lazy": True,
        "member_count": 10 * gi,
        "properties": {"id": fid(g["id"]), "name": f"sunucu-{gi}", "icon": None},
        "channels": [channel(c, i) for i, c in enumerate(cats + rest, 1)],
        "roles": [],
    })
guilds.append({"id": fid("unavailable-guild"), "unavailable": True})

private = []
for pi, c in enumerate(ready["private_channels"][:2], 1):
    private.append({
        "id": fid(c["id"]), "type": c["type"],
        "recipient_ids": [fid(r) for r in c.get("recipient_ids", [])][:2],
        "last_message_id": None,
    })

u = ready["user"]
fixture = {
    "v": ready["v"],
    "user": {"id": fid(u["id"]), "username": "kullanici", "global_name": "Kullanıcı",
             "discriminator": "0", "avatar": None, "bot": False},
    "session_id": "0123456789abcdef0123456789abcdef",
    "session_type": ready["session_type"],
    "resume_gateway_url": "wss://gateway-us-east1-d.discord.gg",
    "guilds": guilds,
    "private_channels": private,
    "read_state": {"entries": [], "partial": False, "version": 1},
}
json.dump(fixture, sys.stdout, ensure_ascii=False, indent=1)
print()
