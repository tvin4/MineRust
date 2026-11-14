import json, base64, sys
from pathlib import Path
T = Path(r"E:\MineRust-data\traces")
B_UUID = "b3b03a00-4714-337e-8103-52c60a6c836f"

MOVES = {"minecraft:move_entity_pos","minecraft:move_entity_pos_rot","minecraft:move_entity_rot",
         "minecraft:rotate_head","minecraft:entity_position_sync","minecraft:set_entity_data",
         "minecraft:set_entity_motion","minecraft:update_attributes","minecraft:set_equipment",
         "minecraft:teleport_entity","minecraft:set_passengers","minecraft:entity_event"}

def varint(b,p=0):
    v=s=0
    while True:
        x=b[p]; p+=1; v|=(x&0x7f)<<s
        if not x&0x80: break
        s+=7
    return v,p

def rows(p):
    out=[]
    for line in open(p, encoding="utf-8", errors="replace"):
        line=line.strip()
        if not line: continue
        r=json.loads(line); r["payload"]=base64.b64decode(r.get("b64") or ""); out.append(r)
    return out

def b_id(lines):
    for r in lines:
        if r.get("name")=="minecraft:add_entity":
            b=r["payload"]
            try: i,p=varint(b)
            except Exception: continue
            raw=b[p:p+16]
            if raw.hex()==B_UUID.replace("-",""): return i
    return None

def about(lines, bid):
    out=[]
    for r in lines:
        if r.get("dir")!="clientbound": continue
        n=r.get("name")
        if n=="minecraft:player_info_update":
            # check uuid present
            b=r["payload"]
            if B_UUID.replace("-","") in b.hex(): out.append(r)
            continue
        if n=="minecraft:player_info_remove":
            if B_UUID.replace("-","") in r["payload"].hex(): out.append(r)
            continue
        if n=="minecraft:remove_entities":
            b=r["payload"]; cnt,p=varint(b); ids=[]
            for _ in range(cnt):
                v,p=varint(b,p); ids.append(v)
            if bid in ids: out.append(r)
            continue
        if n in MOVES:
            try:
                if n=="minecraft:entity_event": v=varint(r["payload"][1:])[0]
                else: v=varint(r["payload"])[0]
            except Exception: continue
            if v==bid: out.append(r)
    return out

def short(n):
    return {"minecraft:move_entity_pos":"pos","minecraft:move_entity_pos_rot":"pos_rot",
            "minecraft:move_entity_rot":"rot","minecraft:rotate_head":"head",
            "minecraft:entity_position_sync":"sync","minecraft:set_entity_data":"meta",
            "minecraft:update_attributes":"attrs","minecraft:remove_entities":"rm",
            "minecraft:player_info_remove":"pi_rm","minecraft:player_info_update":"pi_up",
            "minecraft:add_entity":"add"}.get(n, n)

for code in ("c2","c4"):
    for side in ("", "-ours"):
        p = T/f"two-{code}{side}-A.jsonl"
        lines=rows(p); bid=b_id(lines); ab=about(lines,bid)
        seq=[short(r.get("name")) for r in ab]
        print(f"===== {code}{side} id={bid} n={len(seq)} =====")
        print(" ".join(seq))
        print()
