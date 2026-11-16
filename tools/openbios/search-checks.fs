\ Run line-by-line on the Helios port, with no writable media attached.
\ Compare the native chain walk to the retained Forth implementation, then
\ check independently known values, stack results, flags and search order.
hex
: helios-check 0= abort" HELIOS-OPENBIOS-CHECK-FAILED" ;
variable h-addr variable h-len variable h-wid variable h-xt variable h-flag
: h-miss h-len @ = helios-check h-addr @ = helios-check ;
: h-compare
  h-wid ! h-len ! h-addr !
  h-addr @ h-len @ h-wid @ find-wordlist
  dup h-flag ! if h-xt ! else h-miss then
  h-addr @ h-len @ h-wid @ (find-wordlist-reference)
  dup h-flag @ = helios-check
  if h-xt @ = helios-check else h-miss then
;
get-current constant h-root
wordlist constant h-words
wordlist constant h-other
wordlist constant h-empty
h-words set-current
: Alpha 1234 ;
: AlphaLong 5678 ;
: a 1 ;
: ab 2 ;
: abc 3 ;
: abcd 4 ;
: abcde 5 ;
: abcdef 6 ;
: abcdefg 7 ;
: abcdefgh 8 ;
: zz[ 31 ;
: zz{ 32 ;
: h-shadow 11 ;
: h-immediate 2222 ; immediate
h-other set-current
: h-shadow 22 ;
h-root set-current
" Alpha" h-words h-compare
" aLPHA" h-words h-compare
" AlphaLong" h-words h-compare
" AlphaLon" h-words h-compare
" AlphaLonger" h-words h-compare
" absent" h-words h-compare
" a" h-words h-compare
" aB" h-words h-compare
" aBc" h-words h-compare
" aBcD" h-words h-compare
" aBcDe" h-words h-compare
" aBcDeF" h-words h-compare
" aBcDeFg" h-words h-compare
" aBcDeFgH" h-words h-compare
" ZZ[" h-words h-compare
" ZZ{" h-words h-compare
" Alpha" h-empty h-compare
0 0 h-empty h-compare
0 0 h-words h-compare
0 -1 h-words h-compare
" ALPHA" h-words find-wordlist helios-check execute 1234 = helios-check
" alphaLONG" h-words find-wordlist helios-check execute 5678 = helios-check
" ZZ[" h-words find-wordlist helios-check execute 31 = helios-check
" ZZ{" h-words find-wordlist helios-check execute 32 = helios-check
" h-immediate" h-words search-wordlist 1 = helios-check
execute 2222 = helios-check
" Alpha" h-words search-wordlist -1 = helios-check execute 1234 = helios-check
" absent" h-words search-wordlist 0= helios-check
forth-wordlist h-root h-other h-words 4 set-order
" h-shadow" $find helios-check execute 11 = helios-check
forth-wordlist h-root h-words h-other 4 set-order
" h-shadow" $find helios-check execute 22 = helios-check
h-other set-current
: h-shadow 33 ;
h-root set-current
" h-shadow" h-other h-compare
" h-shadow" $find helios-check execute 33 = helios-check
forth-wordlist h-root 2 set-order
" h-shadow" $find 0= helios-check 2drop
depth 0= helios-check
." HELIOS-OPENBIOS-CHECK-PASSED" cr
