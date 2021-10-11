
#include <iostream>

#include <string.h>
#include <arpa/inet.h> /* htons etc */

/* tests */
#define _USE_MATH_DEFINES
#include <math.h>
#include <climits>

#include "projectrtpfirfilter.h"

lowpass3_4k16k::lowpass3_4k16k() :
  round( 0 ) {
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
  round( 0 ),
  l( ma_length ),
  rtotal( 0 ) {
  this->reset( ma_length );
}

void ma_filter::reset( int packets ) {
  this->l = packets;
  if( this->l >= ma_length ) this->l = ma_length;

  this->rtotal = 0;
  memset( this->history, 0, sizeof( this->history ) );
  this->round = 0;
}

int16_t ma_filter::execute( int16_t val ) {
  this->rtotal -= this->history[ this->round ];
  this->rtotal += val;
  this->history[ this->round ] = val;
  this->round = ( this->round + 1 ) % this->l;

  return this->rtotal / this->l;
}

#ifdef TESTSUITE

/*
TODO - improve
*/
void testlowpass( void ) {
  lowpass3_4k16k lpfilter;

  for( int16_t i = 0; i < 10000; i++ ) {
    lpfilter.execute( i );
  }
}

/*
## testma
*/
void testma( void ) {
  ma_filter ourma;

  for( auto i = 0; i < ma_length; i++ ) {
    ourma.execute( 1 );
  }

  if( 1 != ourma.get() ) throw "Incorrect answer from ma filter";

  for( auto i = 0; i < ( ma_length / 2 ); i++ ) {
    ourma.execute( 100 );
  }

  if( 50 != ourma.get() ) throw "Incorrect answer from ma filter";

  for( auto i = 0; i < ( ma_length / 2 ); i++ ) {
    ourma.execute( 100 );
  }
  if( 100 != ourma.get() ) throw "Incorrect answer from ma filter";
}
#endif

#ifdef NODE_MODULE
/*
Node interface
Notes:

I don't intend these functions to be used in production enviroments. I have added
them so that I can test the filter implimentations. They could be useful for other
purposes but I haven't thought much about the performance of this interface.

To ensure portability I have used network byte order of the buffer array so
values should be set in the array using writeInt16BE
*/

static napi_value createnapibool( napi_env env, bool v ) {
  napi_value result;
  napi_create_uint32( env, v == true? 1 : 0, &result );
  napi_coerce_to_bool( env, result, &result );
  return result;
}

/* Take a buffer of data then pass through a low pass fir filter */
static napi_value filterlowfir( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) {
    napi_throw_error( env, "0", "Unable to get argv" );
    return NULL;
  }

  if( argc != 1 ) {
    napi_throw_error( env, "1", "Incorrect # of arguments" );
    return NULL;
  }

  bool isarray;
  if( napi_ok != napi_is_buffer( env, argv[ 0 ], &isarray ) || !isarray ) {
    napi_throw_error( env, "2", "argv[0] must be a buffer" );
    return NULL;
  }

  size_t bufferlength;
  uint8_t *bufferdata;

  if( napi_ok != napi_get_buffer_info( env, argv[ 0 ], ( void ** ) &bufferdata, &bufferlength ) ) {
    napi_throw_type_error( env, "3", "Couldn't get buffer data" );
    return NULL;
  }

  {
    uint16_t *indata = ( uint16_t * ) bufferdata;
    lowpass3_4k16k filter;

    for( size_t i = 0; i < bufferlength/2; i++ ) {
      indata[ i ] = htons( filter.execute( ntohs( indata[ i ] ) ) );
    }
  }

  return createnapibool( env, true );
}

void initfilter( napi_env env, napi_value &result ) {

  napi_value rtpfilter;
  napi_value nfunction;

  if( napi_ok != napi_create_object( env, &rtpfilter ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "rtpfilter", rtpfilter ) ) return;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, filterlowfir, nullptr, &nfunction ) ) return;
  if( napi_ok != napi_set_named_property( env, rtpfilter, "filterlowfir", nfunction ) ) return;

}

#endif /* NODE_MODULE */
