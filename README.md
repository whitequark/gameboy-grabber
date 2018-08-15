# Game Boy Screen Grabber

TBD

## Glasgow config

    # GameBoy Color
    glasgow run rgb-grabber --port AB --voltage 3.3 --pins-r 8,6,0,9,15 --pins-g 14,1,5,10,13 --pins-b 2,12,4,11,3 --pin-dck 7 --rows 145 --columns 160 --vblank 960e-6

    # GameBoy Advance
    glasgow run rgb-grabber --port AB --voltage 3.3 --pins-r 4,10,5,9,6 --pins-g 13,2,12,3,11 --pins-b 8,15,0,14,1 --pin-dck 7 --rows 161 --columns 240 --vblank 960e-6
