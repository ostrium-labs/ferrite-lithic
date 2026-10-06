"""Outline the existing Plex font for portable lockups; read generated canonical marks."""
from pathlib import Path
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
import re
root=Path(__file__).resolve().parent.parent
font=TTFont(root/'public/fonts/plexmono-latin-bb87500e.woff2')
glyphs=font.getGlyphSet();cmap=font.getBestCmap();units=font['head'].unitsPerEm
size=40;scale=size/units;x=0;paths=[]
for char in 'ferrite-lithic':
 name=cmap[ord(char)];pen=SVGPathPen(glyphs);glyphs[name].draw(pen)
 paths.append(f'<path transform="translate({x:g} 0)" d="{pen.getCommands()}"/>')
 x+=font['hmtx'][name][0]
word=f'<g transform="translate(108 55) scale({scale:g} {-scale:g})">'+''.join(paths)+'</g>'
for variant in ['primary','inverse','mono','mono-inverse']:
 source=(root/f'public/brand/ferrite-mark-{variant}.svg').read_text(encoding='utf-8');body=source[source.index('>')+1:source.rindex('</svg>')];color=re.search(r'stroke="([^"]+)"',source)[1]
 svg=f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 430 88" role="img" aria-label="ferrite-lithic"><g transform="translate(8 8) scale(3)">{body}</g><g fill="{color}">{word}</g></svg>\n'
 (root/f'public/brand/ferrite-logo-{variant}.svg').write_text(svg,encoding='utf-8')
source=(root/'public/brand/ferrite-mark-primary.svg').read_text(encoding='utf-8');body=source[source.index('>')+1:source.rindex('</svg>')];color=re.search(r'stroke="([^"]+)"',source)[1]
width=x*scale
svg=f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 360 204" role="img" aria-label="ferrite-lithic"><g transform="translate(120 8) scale(5)">{body}</g><g fill="{color}" transform="translate({(360-width)/2:g} 180) scale({scale:g} {-scale:g})">'+''.join(paths)+'</g></svg>\n'
(root/'public/brand/ferrite-logo-stacked.svg').write_text(svg,encoding='utf-8')
print('Exported five portable outlined Plex lockups')
