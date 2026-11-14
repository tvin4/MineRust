import json, base64, difflib
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
            if b[p:p+16].hex()==B_UUID.replace("-",""): return i
def about(lines, bid):
    out=[]
    for r in lines:
        if r.get("dir")!="clientbound": continue
        n=r.get("name")
        if n=="minecraft:player_info_update":
            if B_UUID.replace("-","") in r["payload"].hex(): out.append(r)
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
                v=varint(r["payload"][1:])[0] if n=="minecraft:entity_event" else varint(r["payload"])[0]
            except Exception: continue
            if v==bid: out.append(r)
    return out
S={"minecraft:move_entity_pos":"pos","minecraft:move_entity_pos_rot":"pos_rot",
   "minecraft:move_entity_rot":"rot","minecraft:rotate_head":"head",
   "minecraft:entity_position_sync":"sync","minecraft:set_entity_data":"meta",
   "minecraft:update_attributes":"attrs","minecraft:remove_entities":"rm",
   "minecraft:player_info_remove":"pi_rm","minecraft:player_info_update":"pi_up",
   "minecraft:add_entity":"add"}
for code in ("c2","c4"):
    v=about(rows(T/f"two-{code}-A.jsonl"), b_id(rows(T/f"two-{code}-A.jsonl")))
    o=about(rows(T/f"two-{code}-ours-A.jsonl"), b_id(rows(T/f"two-{code}-ours-A.jsonl")))
    vs=[S.get(r.get("name"),"?") for r in v]; os_=[S.get(r.get("name"),"?") for r in o]
    # annotate pos with length to distinguish zero-delta(9)/spawn(8)
    def ann(rows_, seq):
        out=[]
        for r,s in zip(rows_,seq):
            if s=="pos": s=f"pos{len(r['payload'])}"
            out.append(s)
        return out
    vs=ann(v,vs); os_=ann(o,os_)
    print(f"===== {code}: vanilla {len(vs)} ours {len(os_)} =====")
    sm=difflib.SequenceMatcher(a=vs,b=os_,autojunk=False)
    for tag,i1,i2,j1,j2 in sm.get_opcodes():
        if tag=="equal": continue
        print(f"  {tag:8s} van[{i1}:{i2}]={vs[i1:i2]}  ours[{j1}:{j2}]={os_[j1:j2]}")
    print()
