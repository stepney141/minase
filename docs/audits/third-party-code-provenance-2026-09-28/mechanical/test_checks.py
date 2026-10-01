"""Behavioral oracles come from spec.md; fixtures are independent of audited code."""
import unittest
from fractions import Fraction as Q
from text_normalization import decode_strings
from checks import number,norm,phrases,scale_interval,scaled_overlap,local_matches,fingerprint,unit_value,exclusion
class SpecTests(unittest.TestCase):
 def test_literal_forms(self):
  for text,expected in [('0',0),('-1',-1),('0xffu32',255),('0b10_10',10),("1'234ULL",1234),('-3.5e-2',Q(-7,200))]:self.assertEqual(number(text),expected)
 def test_nfkc_identifiers_and_escape(self):
  self.assertEqual(norm('ＦｏｏBar\\nfoo_bar'), 'foo bar foo bar')
 def test_adjacent_strings(self):
  self.assertEqual(decode_strings('"foo" "bar"','cpp'),'foobar')
  self.assertEqual(decode_strings('r###"a\\nb"###','rust'),'a\\nb')
  self.assertEqual(decode_strings('"a\\nb"','rust'),'a\nb')
 def test_m1_boundaries(self):
  self.assertNotIn(('en5','one two three four'),phrases('one two three four',''))
  self.assertIn(('en5','one two three four five'),phrases('one two three four five',''))
  self.assertIn(('ja12','あいうえおかきくけこさし'),phrases('あいうえおかきくけこさし',''))
 def test_protocol_exclusion(self):
  self.assertEqual(exclusion('"setoption name USI_Hash value 256"',('en5','name usi hash value 256'),''),'protocol_specification')
  self.assertEqual(exclusion('"using different storage"',('short','using different storage'),''),'')
 def test_units(self):
  self.assertEqual(unit_value(Q(250),'百分率'),(Q(5,2),'ratio'))
  self.assertEqual(unit_value(Q(2500),'milliseconds'),(Q(5,2),'seconds'))
  self.assertEqual(unit_value(Q(100),'千分率'),(Q(1,10),'ratio'))
 def test_scaling_rounding(self):
  b=list(map(Q,[40,50,100,150,151,152,153,154]));a=list(map(Q,[100,125,250,375,378,380,383,385]));k=scale_interval(a,b)
  self.assertIsNotNone(k);self.assertTrue(all(abs(x-y*k)<=1 for x,y in zip(a,b)))
  self.assertIsNone(scale_interval([Q(-10)]*8,[Q(10)]*8))
 def test_reordered_subset(self):
  b=list(map(Q,[40,50,100,150,151,152,153,154,280]));a=list(map(Q,[385,383,380,378,375,250,125,100,2600]));self.assertEqual(len(scaled_overlap(a,b,Q(5,2))),8)
 def test_winnowing_boundaries_and_locality(self):
  for threshold in [20,40,80]:
   copied=[str(i) for i in range(threshold)];self.assertTrue(local_matches(['prefix']+copied+['suffix'],['other']+copied+['end'],threshold));self.assertFalse(local_matches(copied[:-1],copied[:-1],threshold))
 def test_no_hash_only_matches(self):
  self.assertFalse(local_matches(['a']*100,['b']*100,20))
if __name__=='__main__':unittest.main()
