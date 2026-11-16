\ Run line-by-line at the port's serial Forth prompt (no disks required).
\ Check the legacy packed-string behavior independently of TurboGX FCode.
hex
: helios-check 0= abort" HELIOS-OPENBIOS-CHECK-FAILED" ;
create helios-src ff allot
create helios-buf 102 allot
helios-src ff 33 fill
helios-buf 102 58 fill
" 108000000" helios-buf sun-version1-pack drop
" 1152" helios-buf sun-version1-pack drop
helios-buf c@ 4 = helios-check
helios-buf 4 + c@ 32 = helios-check
helios-buf 5 + c@ 0= helios-check
helios-buf 6 + c@ 30 = helios-check
helios-src ff helios-buf sun-version1-pack drop
helios-buf c@ ff = helios-check
helios-buf ff + c@ 33 = helios-check
helios-buf 100 + c@ 0= helios-check
helios-buf 101 + c@ 58 = helios-check
helios-buf 102 58 fill
0 0 helios-buf sun-version1-pack drop
helios-buf c@ 0= helios-check
helios-buf 1+ c@ 0= helios-check
helios-buf 2 + c@ 58 = helios-check
helios-buf 102 58 fill
" 12" helios-buf pack drop
helios-buf c@ 2 = helios-check
helios-buf 3 + c@ 58 = helios-check
83 get-token drop ' pack = helios-check
." HELIOS-OPENBIOS-CHECK-PASSED" cr
