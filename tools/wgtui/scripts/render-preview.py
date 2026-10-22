#!/usr/bin/env python3
"""Rasterize the demo SVG. Requires Pillow; fonttools/brotli for WOFF2 fonts.

No live WireGuard state is read. No fonts are installed or included in packages.
"""
import argparse
import io
from pathlib import Path
import xml.etree.ElementTree as ET
from PIL import Image, ImageDraw, ImageFont

parser = argparse.ArgumentParser()
parser.add_argument('source', type=Path)
parser.add_argument('destination', type=Path)
parser.add_argument('--font', type=Path, required=True)
parser.add_argument('--font-ext', type=Path)
parser.add_argument('--fallback', type=Path, required=True)
args = parser.parse_args()


def load_font(path, weight):
    if path.suffix == '.woff2':
        from fontTools.ttLib import TTFont
        from fontTools.varLib.instancer import instantiateVariableFont
        font = TTFont(path)
        cmap = font.getBestCmap()
        if 'fvar' in font:
            font = instantiateVariableFont(font, {'wght': weight}, inplace=True)
        font.flavor = None
        stream = io.BytesIO()
        font.save(stream)
        stream.seek(0)
        return ImageFont.truetype(stream, 16), cmap
    return ImageFont.truetype(str(path), 16), None


fonts = {weight: [load_font(path, weight) for path in [args.font, args.font_ext] if path]
         for weight in [400, 600]}
fallback = ImageFont.truetype(str(args.fallback), 16)
svg = ET.parse(args.source).getroot()
image = Image.new('RGB', (int(svg.attrib['width']), int(svg.attrib['height'])), '#090a09')
draw = ImageDraw.Draw(image)
for element in svg.iter():
    if element.tag.endswith('}rect'):
        if element.attrib.get('width') == '100%':
            continue
        x, y = int(element.attrib.get('x', 0)), int(element.attrib.get('y', 0))
        w, h = int(element.attrib['width']), int(element.attrib['height'])
        draw.rectangle((x, y, x+w-1, y+h-1), fill=element.attrib['fill'])
    elif element.tag.endswith('}text'):
        x, y = int(element.attrib['x']), int(element.attrib['y'])
        text, color = element.text or '', element.attrib['fill']
        if text == '─':
            draw.line((x,y-5,x+9,y-5),fill=color)
            continue
        if text == '│':
            draw.line((x+5,y-16,x+5,y+5),fill=color)
            continue
        if text in {'╭','╮','╰','╯'}:
            cx,cy,r=x+5,y-5,4
            if text=='╭':
                draw.line((cx,cy+r,cx,y+5),fill=color)
                draw.line((cx+r,cy,x+9,cy),fill=color)
                draw.arc((cx,cy,cx+r*2,cy+r*2),180,270,fill=color)
            elif text=='╮':
                draw.line((x,cy,cx-r,cy),fill=color)
                draw.line((cx,cy+r,cx,y+5),fill=color)
                draw.arc((cx-r*2,cy,cx,cy+r*2),270,360,fill=color)
            elif text=='╰':
                draw.line((cx,y-16,cx,cy-r),fill=color)
                draw.line((cx+r,cy,x+9,cy),fill=color)
                draw.arc((cx,cy-r*2,cx+r*2,cy),90,180,fill=color)
            else:
                draw.line((cx,y-16,cx,cy-r),fill=color)
                draw.line((x,cy,cx-r,cy),fill=color)
                draw.arc((cx-r*2,cy-r*2,cx,cy),0,90,fill=color)
            continue
        weight=int(element.attrib.get('font-weight',400))
        font=next((font for font,cmap in fonts[weight]
                   if cmap is None or all(ord(c) in cmap for c in text)),fallback)
        draw.text((x,y),text,fill=color,font=font,anchor='ls')
        if element.attrib.get('text-decoration')=='underline':
            draw.line((x,y+2,x+9,y+2),fill=color)
image.save(args.destination)
print(args.destination)
