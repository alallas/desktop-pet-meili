from PIL import Image, ImageDraw
import os
T = os.path.join(os.environ["TEMP"], "clips")
names = ["1 idle_a","2 wave","3 jump","4 sit","5 sleep","6 petted","7 listen","8 rub_face","9 gift_shell","0 explain"]
crops = []
for k in "1234567890":
    im = Image.open(os.path.join(T, f"clip_{k}.png"))
    crops.append(im.crop((0, 584, 600, 1504)))
w, h = crops[0].size
scale = 0.42
tw, th = int(w*scale), int(h*scale)
sheet = Image.new("RGB", (tw*5+60, th*2+90), (30,30,40))
d = ImageDraw.Draw(sheet)
for i, c in enumerate(crops):
    r, col = divmod(i, 5)
    x, y = col*(tw+10)+10, r*(th+40)+30
    sheet.paste(c.resize((tw, th)), (x, y))
    d.text((x, y-20), names[i], fill=(255,255,120))
out = r"D:\Repos\desktop-pet-meili\art\raw\clip_review_grid.png"
sheet.save(out)
print("saved", out, sheet.size)
