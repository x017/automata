def update(entity, ctx):
    a = entity.attributes
    if "done" in a:
        return
    a["int"] = a["int"] + 1
    a["float"] = 2.5
    a["text"] = "hi " + a["text"]
    a["flag"] = not a["flag"]
    a["vec"] = (1, 2, 3)
    a["color"] = (0.1, 0.2, 0.3, 1.0)
    a["seen_vec"] = a["start_vec"]
    del a["gone"]
    a.setdefault("defaulted", 7)
    a["done"] = True
