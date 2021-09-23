

#include <iostream>

#define _USE_MATH_DEFINES
#include <math.h>

#include <string>
#include <boost/algorithm/string.hpp>

#include "projectrtpsoundfile.h"
#include "projectrtptonegen.h"

typedef std::vector< std::string > vectorofstrings;

/*
We need to be able to generate tones. This is following the standard: https://www.itu.int/dms_pub/itu-t/opb/sp/T-SP-E.180-2010-PDF-E.pdf. Looping will be handled by soundsoup. So this section only needs to handle one cycle of the tone. We allocate memory required to generate the tone at the sample rate but not completley!

Our goal is to be efficient, so we do not generate this on the fly - most tones will be generated into wav files and played when required.

If we want to play a tone continuously we should find a nicely looped file (e.g 1S will mean all frequencies in the file will hit zero at the end of the file). This would simplify our generation.

In the standard we have definitions such as:

United Kingdom of Great Britain
and Northern Ireland
Busy tone - 400 0.375 on 0.375 off
Congestion tone - 400 0.4 on 0.35 off 0.225 on 0.525 off
Dial tone - 50//350+440 continuous
Number unobtainable tone - 400 continuous
Pay tone - 400 0.125 on 0.125 off
Payphone recognition tone - 1200/800 0.2 on 0.2 off 0.2 on 2.0 off
Ringing tone - 400+450//400x25//400x16 2/3 0.4 on 0.2 off 0.4 on 2.0 off

i.e. Tone - Frequency - Cadence

The frequency is

Frequency in Hz:
f1×f2 f1 is modulated by f2
f1+f2 the juxtaposition of two frequencies f1 and f2 without modulation
f1/f2 f1 is followed by f2
f1//f2 in some exchanges frequency f1 is used and in others frequency f2 is used.
Cadence in seconds: ON – OFF

Try to keep our definitions as close to the standard. We also have to introduce some other items:

* Amplitude
* Change (in frequency or amplitude) - frequency can be handled by modulated

Take ringing tone:

400+450//400x25//400x16 2/3 0.4 on 0.2 off 0.4 on 2.0 off

We can ignore the // in our definition as we can simply choose the most common one.
So either 400+450 or 400x25
Three does not appear ot be anything in the standard relating to the 2/3?

Amplitude can be introduced by *
so

400+450 becomes 400+450*0.75 (every frequency will have its amplitude reduced).
400x25*0.75 is then also suported.

Increasing tones such as:
950/1400/1800

Cadence
950/1400/1800/0:333/333/333/1000
Note, we have introduced a final /0 to indicate silence. The cadences will iterated through for every / in the frequency list and is in mS (the standard lists in seconds). We don't need to support loops as soundsoup supports loops.
For:
950/1400/1800/0:333
Means each section will be 333mS.

Change
400+450*0.75~0 will reduce the amplitude from 0.75 to 0 during that cadence period
400~450 will increase the frequency during that cadence period

Note 400+450x300 is not supported.

*/

static void gentone( int16_t *outbuffer, int sizeofblock, double startfrequency, double endfrequency, double startamp, double endamp, int samplerate )
{
  if( 0 == startfrequency && 0 == endfrequency ) return;

  double angle;
  double ampatpos = startamp;
  double amppersample = ( endamp - startamp ) / sizeofblock;
  double freqpersample = ( endfrequency - startfrequency ) / sizeofblock;
  double frequencyatpos = startfrequency;
  if( 0 == startfrequency )
  {
    angle = 0;
  }
  else
  {
    angle = ( 2 * M_PI ) / ( double ) samplerate * startfrequency;
  }

  for( int i = 0; i < sizeofblock; i++ )
  {
    *outbuffer += static_cast< int16_t >( ( sin( angle * i ) * SHRT_MAX * ampatpos ) );
    outbuffer++;

    ampatpos += amppersample;
    frequencyatpos += freqpersample;

    if( 0 == frequencyatpos )
    {
      angle = 0;
    }
    else
    {
      angle = ( 2 * M_PI ) / ( double ) samplerate * frequencyatpos;
    }
  }
}


static void gen( std::string tone, std::string filename )
{
  vectorofstrings freqscadence;
  boost::split( freqscadence, tone, boost::is_any_of( ":" ) );

  if( 2 != freqscadence.size() )
  {
    std::cerr << "You must supply a cadence, eg. 400:1000" << std::endl;
    return;
  }

  vectorofstrings frequencies;
  vectorofstrings cadences;
  boost::split( frequencies, freqscadence[ 0 ], boost::is_any_of( "/" ) );
  boost::split( cadences, freqscadence[ 1 ], boost::is_any_of( "/" ) );

  uint8_t *readbuffer = nullptr;
  wavheader outwavheader;
  initwav( &outwavheader );

  /* 1. Calculate total time */
  vectorofstrings::iterator it;
  vectorofstrings cadenceparts;
  int cadencepos = 0;
  int cadencetotal = 0;
  for( it = frequencies.begin(); it != frequencies.end(); it++ )
  {
    cadencetotal += std::atoi( cadences[ cadencepos ].c_str() );
    cadencepos = ( cadencepos + 1 ) % cadences.size();
  }

  std::cout << "Total time is " << cadencetotal << "mS" << std::endl;

  /* 2. Allocate enough memory */
  outwavheader.subchunksize = cadencetotal * outwavheader.sample_rate / 1000 * 2 /* 2 bytes per sample */;
  outwavheader.chunksize = outwavheader.subchunksize + 36;

  readbuffer = new uint8_t[ outwavheader.chunksize ];
  memset( readbuffer, 0, outwavheader.chunksize );

  /* 3. Generate tones */
  int pos = 0;
  cadencepos = 0;
  for( it = frequencies.begin(); it != frequencies.end(); it++ )
  {
    /* Current cadence. */
    int cadence = std::atoi( cadences[ cadencepos ].c_str() );
    cadencepos = ( cadencepos + 1 ) % cadences.size();

    int16_t *outbuffer = ( int16_t * ) &readbuffer[ pos * 2 ];
    int sizeofblock = outwavheader.sample_rate * cadence / 1000 /*mS*/;

    /* current frequency */
    /* *it could be 400+450 or or 400x25 450+450*0.75 or 450~480 or 450+450*0.75~1 */
    vectorofstrings freqamp;
    boost::split( freqamp, *it, boost::is_any_of( "*" ) );
    double startamp = 1;
    double endamp = 1;
    if( freqamp.size() > 1 )
    {
      vectorofstrings ampfromto;
      boost::split( ampfromto, freqamp[ 1 ], boost::is_any_of( "~" ) );
      startamp = std::atof( ampfromto[ 0 ].c_str() );
      endamp = startamp;
      if( ampfromto.size() > 1 )
      {
        endamp = std::atof( ampfromto[ 1 ].c_str() );
      }
    }
    vectorofstrings freqparts;
    boost::split( freqparts, freqamp[ 0 ], boost::is_any_of( "+x~" ) );

    double startfreq = -1;
    double endfreq = -1;

    std::size_t found = freqamp[ 0 ].find_first_of( "+x~" );
    if( std::string::npos == found )
    {
      startfreq = std::atof( freqparts[ 0 ].c_str() );
      gentone( outbuffer, sizeofblock, startfreq, startfreq, startamp, endamp, outwavheader.sample_rate );
      goto continueloop;
    }
    else
    {
      switch( freqamp[ 0 ][ found ] )
      {
        case '+':
        {
          for( auto freqit = freqparts.begin(); freqit != freqparts.end(); freqit++ )
          {
            startfreq = std::atof( freqit->c_str() );
            gentone( outbuffer, sizeofblock, startfreq, startfreq, startamp, endamp, outwavheader.sample_rate );
          }
          break;
        }
        case '~':
        {
          startfreq = std::atof( freqparts[ 0 ].c_str() );
          endfreq = std::atof( freqparts[ 1 ].c_str() );
          gentone( outbuffer, sizeofblock, startfreq, endfreq, startamp, endamp, outwavheader.sample_rate );
          break;
        }
      }
    }

continueloop:
    pos += sizeofblock;
  }


  /* Write */
  int file = open( filename.c_str(), O_RDWR | O_CREAT, S_IRUSR | S_IWUSR );
  __off_t position = lseek( file, 0, SEEK_END );
  if( 0 == position )
  {
    write( file, &outwavheader, sizeof( wavheader ) );
    write( file, readbuffer, outwavheader.chunksize );
  }
  else
  {
    wavheader currentheader;
    lseek( file, 0, SEEK_SET );
    read( file, &currentheader, sizeof( wavheader ) );
    lseek( file, 0, SEEK_END );

    if( currentheader.audio_format == outwavheader.audio_format &&
          currentheader.sample_rate == outwavheader.sample_rate )
    {
      /* Ok, good enough! */
      currentheader.chunksize += outwavheader.chunksize;
      currentheader.subchunksize += outwavheader.subchunksize;

      lseek( file, 0, SEEK_SET );
      write( file, &currentheader, sizeof( wavheader ) );
      lseek( file, 0, SEEK_END );
      write( file, readbuffer, outwavheader.chunksize );
    }
    else
    {
      std::cerr << "File format to append should be the same" << std::endl;
    }
  }

  /* Clean up */
  if( nullptr != readbuffer )
  {
    delete[] readbuffer;
  }

  close( file );
}

/*
Our node functions.
*/
static napi_value createnapibool( napi_env env, bool v ) {
  napi_value result;
  napi_create_uint32( env, v == true? 1 : 0, &result );
  napi_coerce_to_bool( env, result, &result );
  return result;
}

static napi_value tonegen( napi_env env, napi_callback_info info ) {
  size_t argc = 2;
  napi_value argv[ 2 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( 2 != argc ) {
    napi_throw_error( env, "0", "You must provide tone.generate( toneformat /*e.g. 350+440*0.5:1000*/, filename )" );
    return NULL;
  }

  size_t bytescopied;
  char tonedcescription[ 256 ];
  char filename[ 256 ];

  napi_get_value_string_utf8( env, argv[ 0 ], tonedcescription, sizeof( tonedcescription ), &bytescopied );
  if( 0 == bytescopied || bytescopied >= sizeof( tonedcescription ) ) {
    napi_throw_error( env, "1", "Tone definition bad or too long" );
    return NULL;
  }

  napi_get_value_string_utf8( env, argv[ 1 ], filename, sizeof( filename ), &bytescopied );
  if( 0 == bytescopied || bytescopied >= sizeof( filename ) ) {
    napi_throw_error( env, "1", "Filename too long" );
    return NULL;
  }

  gen( tonedcescription, filename );

  return createnapibool( env, true );
}

void inittonegen( napi_env env, napi_value &result ) {
  napi_value tonegenobj;
  napi_value nfunction;

  if( napi_ok != napi_create_object( env, &tonegenobj ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "tone", tonegenobj ) ) return;
  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, tonegen, nullptr, &nfunction ) ) return;
  if( napi_ok != napi_set_named_property( env, tonegenobj, "generate", nfunction ) ) return;

}
