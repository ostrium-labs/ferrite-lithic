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

# The repository README uses a compact vector banner: no external font or raster image.
banner_mark=(root/'public/brand/ferrite-mark-primary.svg').read_text(encoding='utf-8')
mark_body=banner_mark[banner_mark.index('>')+1:banner_mark.rindex('</svg>')]
def outlined(text,px,x,y,fill):
    advance=px/units
    glyphs_out=[];offset=0
    for char in text:
        name=cmap[ord(char)];pen=SVGPathPen(glyphs);glyphs[name].draw(pen)
        glyphs_out.append(f'<path transform="translate({offset:g} 0)" d="{pen.getCommands()}"/>')
        offset+=font['hmtx'][name][0]
    return f'<g transform="translate({x} {y}) scale({advance:g} {-advance:g})" fill="{fill}">'+''.join(glyphs_out)+'</g>'
ink='#14181a';paper='#e9ebe9';soft='#4e5754';signal='#a34816'
banner=(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1400 300" role="img" aria-labelledby="title desc">'
        f'<title id="title">ferrite-lithic — Hardware you write in Rust, as real gates</title>'
        f'<desc id="desc">One graph. Two backends. One clock.</desc>'
        f'<rect width="1400" height="300" fill="{paper}"/>'
        f'<path d="M32 34H1368M32 266H1368" stroke="{ink}" stroke-opacity=".18"/>'
        f'<g fill="{ink}" fill-opacity=".045"><path d="M32 50H1368M32 250H1368" stroke="{ink}" stroke-opacity=".12"/></g>'
        f'<g transform="translate(70 76) scale(6)">{mark_body}</g>'
        f'{outlined("ferrite-lithic",58,270,158,ink)}'
        f'{outlined("Hardware you write in Rust, as real gates.",21,274,207,soft)}'
        f'{outlined("One graph. Two backends. One clock.",18,274,242,signal)}'
        f'<path d="M1030 62V238" stroke="{ink}" stroke-opacity=".3"/>'
        f'{outlined("GRAPH",14,1080,103,soft)}{outlined("1",18,1314,103,ink)}'
        f'{outlined("BACKENDS",14,1080,153,soft)}{outlined("2",18,1314,153,ink)}'
        f'{outlined("CLOCK",14,1080,203,soft)}{outlined("1",18,1314,203,ink)}'
        '</svg>\n')
(root/'public/brand/readme-banner.svg').write_text(banner,encoding='utf-8')
print('Exported five outlined Plex lockups and the README banner')
