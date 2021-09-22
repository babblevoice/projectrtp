
#include <iostream>

#include <string.h>

/* tests */
#define _USE_MATH_DEFINES
#include <math.h>
#include <climits>

#include "firfilter.h"

lowpass3_4k16k::lowpass3_4k16k() {
  this->reset();
}

void lowpass3_4k16k::reset( void ) {
  memset( this->history, 0, sizeof( this->history ) );
  this->round = 0;
}

/*!md
# lowpass3_4k16k::execute
Implements a 3.4Khz low pass filter on 16k sampling rate (used for downsampln 16K to 8K data).

y[n] = b0x[n]... for each coefficient.
This means we do have to maintain history as we need b points of history from the last packet (otherwise this is likely to introduce noise).
16K sampling means we can have frequencies of up to 7KHz, G711 with an 8K sampling rate gos up to 3.4KHz.
So to downsample, we have to filter out frequencies we don't want then pick the interleaved samples.

fir1 (18, 0.5) in octave which drops off at around 3.4KHz with 16K sampling.
[ 0.00282111, -0.00010493, -0.00850740, 0.00030192, 0.02921139, -0.00060373, -0.08147804, 0.00086913, 0.30864825,
  0.49768460, 0.30864825, 0.00086913, -0.08147804, -0.00060373, 0.02921139, 0.00030192, -0.00850740, -0.00010493,
  0.00282111 ];

fir1 (12, 0.6) also appears to have an ok responce, plus has the benefit of smaller filter.
[ -0.00407771, 0.00013892, 0.02346967, -0.03425401, -0.07174015, 0.28561976, 0.60168706, 0.28561976, -0.07174015,
  -0.03425401, 0.02346967, 0.00013892, -0.00407771 ];

Normalized frequency xpi rad per sample

http://www.arc.id.au/FilterDesign.html is also a good tool. Kaiser-Bessel filter designer. 0-3.4KHz, target 50dB. 16K sampling.


Works nicely in a spreadsheet:
= (-0.002102 * M17) + ( 0.000519 * M16 ) + ( 0.014189 * M15 ) + ( 0.010317 * M14 ) + ( -0.037919 * M13 ) + ( -0.060378 * M12 ) + (0.063665* M11) + (0.299972* M10) + (0.425000* M9) + (0.299972* M8) + (0.063665* M7) + (-0.060378* M6) + (-0.037919* M5) + (0.010317* M4) + (0.014189* M3) + (0.000519* M2) + (-0.002102* M1)

*/

static float lp3_4k16kcoeffs[ lowpass3_4k16kfl ] = {
                  -0.002102,
                  0.000519,
                  0.014189,
                  0.010317,
                  -0.037919,
                  -0.060378,
                  0.063665,
                  0.299972,
                  0.425000,
                  0.299972,
                  0.063665,
                  -0.060378,
                  -0.037919,
                  0.010317,
                  0.014189,
                  0.000519,
                  -0.002102 };

int16_t lowpass3_4k16k::execute( int16_t val ) {
  float runtot = 0;
  int j = this->round;

  this->history[ j ] = val;

  int i = 0;
  for ( j = ( j + 1 ) % lowpass3_4k16kfl;  j < lowpass3_4k16kfl;  j++ ) {
    runtot += lp3_4k16kcoeffs[ i ] * this->history[ j ];
    i++;
  }

  j = 0;

  for ( ;  i < lowpass3_4k16kfl;  i++ ) {
    runtot += lp3_4k16kcoeffs[ i ] * this->history[ j ];
    j++;
  }

  this->round = ( this->round + 1 ) % lowpass3_4k16kfl;
  return ( int16_t ) runtot;
}

/* Moving Average filter */
ma_filter::ma_filter():
  l( ma_length ),
  rtotal( 0 ) {
  this->reset( ma_length );
}

void ma_filter::reset( int seconds ) {
  this->l = seconds * 50;
  if( this->l >= ma_length ) this->l = ma_length;

  this->rtotal = 0;
  memset( this->history, 0, sizeof( this->history ) );
}

int16_t ma_filter::execute( int16_t val ) {
  this->rtotal -= this->history[ this->round ];
  this->rtotal += val;
  this->history[ this->round ] = val;
  this->round = ( this->round + 1 ) % this->l;

  return this->rtotal / this->l;
}

/*
## testma

*/
void testma( void ) {
  std::cout << "Moving average test" << std::endl;
  ma_filter ourma;

  for( auto i = 0; i < ma_length; i++ ) {
    ourma.execute( 1 );
  }

  std::cout << "Latest val after pumping in 1: " << ourma.get() << std::endl;

  for( auto i = 0; i < ( ma_length / 2 ); i++ ) {
    ourma.execute( 100 );
  }
  std::cout << "Latest val after half filling with 100: " << ourma.get() << std::endl;

  for( auto i = 0; i < ( ma_length / 2 ); i++ ) {
    ourma.execute( 100 );
  }
  std::cout << "Latest val after half filling with 100: " << ourma.get() << std::endl;
}

/*
## testlofir
Call with frequency to generate a frequency then apply the filter to see the responce.
*/
void testlofir( int frequency ) {
  lowpass3_4k16k filter;
  /* 1 seconds worth - 16K sampling */

  /*
    1Hz = (2pi/16000)
    angle += (2 * M_PI) / 16000;
  */

  for( int i = 0; i < 16000; i++ ) {
    int16_t amp = sin( ( ( 2.0 * M_PI ) / 16000.0 ) * i * frequency ) * 20000;
    //std::cout << amp << std::endl;
    std::cout << filter.execute( amp ) << std::endl;
  }
}
