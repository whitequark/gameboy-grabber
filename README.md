# Game Boy Screen Grabber

TBD

## Glasgow config

    # GameBoy Color
    glasgow run rgb-grabber --port AB --voltage 3.3 --pins-r 8,6,0,9,15 --pins-g 14,1,5,10,13 --pins-b 2,12,4,11,3 --pin-dck 7 --rows 145 --columns 160 --vblank 960e-6

    # GameBoy Advance
    glasgow run rgb-grabber --port AB --voltage 3.3 --pins-r 4,10,5,9,6 --pins-g 13,2,12,3,11 --pins-b 8,15,0,14,1 --pin-dck 7 --rows 161 --columns 240 --vblank 960e-6

A0  0 B3
A1  1 B5
A2  2 G2
A3  3 G4
A4  4 R1
A5  5 R3
A6  6 R5
A7  7 DCK
B0  8 B1
B1  9 R4
B2 10 R2
B3 11 G5
B4 12 G3
B5 13 G1
B6 14 B4
B7 15 B2
