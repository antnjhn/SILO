import glob
import os
import re

SRC = 'src/renderer/js'
js = "\n".join(
    open(p, encoding='utf-8', newline='').read()
    for p in sorted(glob.glob(os.path.join(SRC, '*.js')))
)

# class="..." / class='...' / className = "..." assignments in template strings
classes = set()
for m in re.finditer(r'class\s*=\s*(["\'])([^"\']*)\1', js):
    for c in m.group(2).split():
        classes.add(c)
for m in re.finditer(r'classList\.(?:add|remove|toggle)\(([^)]*)\)', js):
    for tok in re.finditer(r'["\']([a-z0-9 _-]+)["\']', m.group(1)):
        for c in tok.group(1).split():
            classes.add(c)

print('=== classes referenced in the renderer JS ===')
print(' '.join(sorted(classes)))
print()
ids = set(re.findall(r'getElementById\(["\']([^"\']+)["\']\)', js))
print('=== ids referenced in the renderer JS (for reference) ===')
print(len(ids), 'ids')
