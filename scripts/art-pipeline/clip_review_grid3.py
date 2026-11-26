from PIL import Image, ImageDraw
import os
T = r"C:\Users\79879\AppData\Local\Temp\clips3"
names = {"1":"idle_a","2":"wave","3":"jump","4":"sit","5":"sleep","6":"petted","7":"listen","8":"rub_face","9":"gift_shell","0":"explain"}
BOX = (0, 584, 600, 1504)
crops = {}
for k, nm in names.items():
    for f in "abc":
        p = os.path.join(T, f"clip_{k}_{f}.png")
        if os.path.exists(p):
            im = Image.open(p).crop(BOX)
            crops[(k, f)] = im.resize((int(600*0.36), int(920*0.36)))
tw, th = crops[("1","a")].size
# 10 clip 各取中段帧 b（无 b 则 a），两行五列
order = ["1","2","3","4","5","6","7","8","9","0"]
sheet = Image.new("RGB", (tw*5+60, th*2+90), (30,30,40))
d = ImageDraw.Draw(sheet)
for i, k in enumerate(order):
    r, col = divmod(i, 5)
    x, y = col*(tw+10)+10, r*(th+40)+30
    c = crops.get((k,"b")) or crops.get((k,"a"))
    sheet.paste(c, (x, y))
    d.text((x, y-18), f"{k} {names[k]}", fill=(255,255,120))
out = r"D:\Repos\desktop-pet-meili\art\raw\clip_review_grid3.png"
sheet.save(out)
print("saved", out, sheet.size)
