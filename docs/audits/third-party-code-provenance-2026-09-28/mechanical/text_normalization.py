"""Decode and concatenate already lexer/AST-extracted string nodes."""
import re

def unescape(s):
 def convert(m):
  t=m.group(1)
  basic={'n':'\n','r':'\r','t':'\t','b':'\b','f':'\f','v':'\v','a':'\a','0':'\0','\\':'\\','"':'"',"'":"'",'\n':''}
  if t in basic:return basic[t]
  if t.startswith('u{'):return chr(int(t[2:-1],16))
  if t.startswith(('u','U','x')):return chr(int(t[1:],16))
  if re.fullmatch('[0-7]{1,3}',t):return chr(int(t,8))
  return '\\'+t
 return re.sub(r'\\(u\{[0-9a-fA-F]+\}|u[0-9a-fA-F]{4}|U[0-9a-fA-F]{8}|x[0-9a-fA-F]{2}|[0-7]{1,3}|.)',convert,s,flags=re.S)

def decode_strings(raw,language):
 i=0;out=[]
 while i<len(raw):
  while i<len(raw) and raw[i].isspace():i+=1
  cpp=re.match(r'(?:u8|u|U|L)?R"([^ ()\\\t\r\n]*)\(',raw[i:]) if language in ['c','cpp'] else None
  if cpp:
   begin=i+len(cpp.group());closing=')'+cpp.group(1)+'"';end=raw.find(closing,begin)
   if end<0:return raw
   out.append(raw[begin:end]);i=end+len(closing);continue
  m=re.match(r'(?:br|rb|fr|rf|[rRbBuUfFL])?(?:#+)?(?:\"\"\"|\'\'\'|\"|\'|`)',raw[i:])
  if not m:return raw # Interpolated/nonliteral syntactic wrapper remains explicit.
  opening=m.group();i+=len(opening)
  quote='"""' if opening.endswith('"""') else "'''" if opening.endswith("'''") else opening[-1]
  prefix=opening[:-len(quote)];hashes=prefix.count('#');closing=quote+'#'*hashes;israw='r' in prefix.lower() or language=='scala' and len(quote)==3
  begin=i
  while i<len(raw):
   if raw.startswith(closing,i):break
   if raw[i]=='\\' and not israw:i+=2
   else:i+=1
  value=raw[begin:i];out.append(value if israw else unescape(value));i+=len(closing)
 return ''.join(out)
