
/* needed to build on ubuntu */
#include <utility>

#include <iostream>
#include <algorithm>

#include "projectrtpsoundfile.h"
#include "globals.h"


constexpr size_t AIOCBSINGLECHANBUFFNUMSAMPLES = L1616PAYLOADSAMPLES;
constexpr size_t AIOCBBUFFERNUMSAMPLES = ( AIOCBSINGLECHANBUFFNUMSAMPLES * MAXNUMBEROFCHANNELS );

/*
soundfile
*/
soundfile::soundfile( int fromfile ) :
  file( fromfile ),
  url(),
  ourwavheader(),
  currentcbindex( 0 ),
  cbwavheader(),
  buffer(),
  filelock( true /* initialised as locked */ ) {

  /* As it is asynchronous - we read wav header + ahead */
  memset( &this->cbwavheader, 0, sizeof( aiocb ) );
  this->cbwavheader.aio_nbytes = sizeof( wavheader );
  this->cbwavheader.aio_fildes = file;
  this->cbwavheader.aio_offset = 0;
  this->cbwavheader.aio_buf = &this->ourwavheader;

  off_t fileoffset = sizeof( wavheader );
  for( auto i = 0; i < SOUNDFILENUMBUFFERS; i++ ) {
    soundbuffer &thisbuffer = this->buffer[ i ];
    aiocb &thisblock = this->cbwavblock[ i ];

    thisbuffer.reserve( AIOCBBUFFERNUMSAMPLES );

    memset( &thisblock, 0, sizeof( aiocb ) );
    this->cbwavblock[ i ].aio_fildes = this->file;

    /* These 2 values are modified depending on format and num channels */
    thisblock.aio_nbytes = AIOCBBUFFERNUMSAMPLES * sizeof( uint16_t );
    thisblock.aio_offset = fileoffset;
    fileoffset += thisblock.aio_nbytes;

    thisblock.aio_buf = thisbuffer.data();
  }
}

soundfile::~soundfile() {

  SpinLockGuard guard( this->filelock );

  if ( -1 != this->file ) {
    /* Is there a better way? Not waiting for the cancel can can cause the async
    opperation to write to our buffer which is much worse */
    while( AIO_NOTCANCELED == aio_cancel( this->file, NULL ) );
    close( this->file );
    this->file = -1;
  }
}

uint8_t soundfile::getwavformattopt( void ) {

  if( 8000 == this->ourwavheader.sample_rate ) {
    return L168KPAYLOADTYPE;
  }
  return L1616KPAYLOADTYPE;
}

int soundfile::getsampleratefrompt( uint8_t pt ) {
  if( G722PAYLOADTYPE == pt ) {
    return 16000;
  }
  return 8000;
}

uint32_t soundfile::getwriteduration( void ) /* mS */ {
  return this->ourwavheader.subchunksize / this->ourwavheader.byte_rate;
}



/*
## soundfile
Time to simplify. We will read wav files - all should be in pcm format - either
wideband or narrow band. Anything else we will throw out. In the future we may
support pre-encoded - but for now...

We need to support
* read and write (play and record)
* whole read and store in memory (maybe)
* looping a file playback (this is effectively moh)
* multiple readers (of looped files only - equivalent of moh)
* virtual files (i.e. think tone://) ( think tone_stream in FS - work on real first)
* Need to review: https://www.itu.int/dms_pub/itu-t/opb/sp/T-SP-E.180-2010-PDF-E.pdf
* Also: TGML - https://freeswitch.org/confluence/display/FREESWITCH/TGML I think a simpler version may be possible

Further work needed on buffers. I started implimenting multiple read buffers - but there is only 1
aiocb - which breaks that concept. As we only read/write on a ticket we can probably silently fail
perhaps monitor (std::cerr).
*/
soundfilereader::soundfilereader( std::string &url ) :
  soundfile( open( url.c_str(), O_RDONLY | O_NONBLOCK, 0 ) ),
  bytecount( L16WIDEBANDBYTES ),
  samplecount( L1616PAYLOADSAMPLES ),
  badheader( false ),
  headerread( false ),
  bodyread( false ),
  initseekmseconds( 0 ),
  ploadtype( L168KPAYLOADTYPE ) {

  if ( -1 == this->file ) {
    /* Not much more we can do */
    releasespinlock( this->filelock );
    return;
  }

  /* As it is asynchronous - we read wav header + ahead */
  memset( &this->cbwavheader, 0, sizeof( aiocb ) );
  this->cbwavheader.aio_nbytes = sizeof( wavheader );
  this->cbwavheader.aio_fildes = this->file;
  this->cbwavheader.aio_offset = 0;
  this->cbwavheader.aio_buf = &this->ourwavheader;

  for( auto i = 0; i < SOUNDFILENUMBUFFERS; i++ ) {
    aiocb &thisblock = this->cbwavblock[ i ];
    soundbuffer &thisbuffer = this->buffer[ i ];

    thisbuffer.resize( this->samplecount );
    std::fill_n( thisbuffer.begin(), thisbuffer.size(), 0 );

    thisblock.aio_nbytes = this->bytecount;
    thisblock.aio_fildes = this->file;
    thisblock.aio_offset = sizeof( wavheader ) + ( i * this->bytecount );
    thisblock.aio_buf = thisbuffer.data();
  }

  /* read */
  if ( aio_read( &this->cbwavheader ) == -1 ) {
    fprintf( stderr, "aio_read read of header failed in soundfile\n" );
    this->badheader = true;
    releasespinlock( this->filelock );
    return;
  }

  releasespinlock( this->filelock );
	return;
}

/*
# d-stor
Clean up.
*/
soundfilereader::~soundfilereader() {
}

/*
# create
Shared pointer version of us.
*/
soundfilereader::pointer soundfilereader::create( std::string url ) {
  return pointer( new soundfilereader( url ) );
}

void soundfilereader::parseheader( void ) {
  if( this->headerread ) return;

  if( aio_error( &this->cbwavheader ) == EINPROGRESS ) {
    fprintf( stderr, "Read of soundfile wav header has not completed\n" );
    return;
  }

  this->headerread = true;

  if( 'W' != this->ourwavheader.wave_header[ 0 ] ) {
    this->badheader = true;
  }

  switch( this->ourwavheader.sample_rate ) {
    case 8000:
    case 16000:
      break;
    default:
      fprintf( stderr, "Bad sample rate in wav\n" );
      this->badheader = true;
      return;
  }

  this->ploadtype = L168KPAYLOADTYPE;
  this->bytecount = L16NARROWBANDBYTES;
  this->samplecount = L16PAYLOADSAMPLES;
  switch( this->ourwavheader.audio_format ) {
    case WAVE_FORMAT_PCM: {
      if( 8000 == this->ourwavheader.sample_rate ) {
        this->ploadtype = L168KPAYLOADTYPE;
        this->bytecount = L16NARROWBANDBYTES;
        this->samplecount = L16PAYLOADSAMPLES;
      } else if( 16000 == this->ourwavheader.sample_rate ) {
        this->ploadtype = L1616KPAYLOADTYPE;
        this->bytecount = L16WIDEBANDBYTES;
        this->samplecount = L1616PAYLOADSAMPLES;
      } else {
        this->badheader = true;
        return;
      }
      break;
    }
    case WAVE_FORMAT_ALAW: {
      this->ploadtype = PCMAPAYLOADTYPE;
      this->bytecount = G711PAYLOADBYTES;
      this->samplecount = G711PAYLOADSAMPLES;
      break;
    }
    case WAVE_FORMAT_MULAW: {
      this->ploadtype = PCMUPAYLOADTYPE;
      this->bytecount = G711PAYLOADBYTES;
      this->samplecount = G711PAYLOADSAMPLES;
      break;
    }
    case WAVE_FORMAT_POLYCOM_G722: {
      this->ploadtype = G722PAYLOADTYPE;
      this->bytecount = G722PAYLOADBYTES;
      this->samplecount = G722PAYLOADSAMPLES;
      break;
    }
    case WAVE_FORMAT_GLOBAL_IP_ILBC: {
      this->ploadtype = ILBCPAYLOADTYPE;
      this->bytecount = ILBC20PAYLOADBYTES;
      this->samplecount = ILBC20PAYLOADSAMPLES;
      break;
    }
    default: {
      this->badheader = true;
      fprintf( stderr, "Bad audio format in wav\n" );
    }
  }

  auto thisindex = this->currentcbindex % SOUNDFILENUMBUFFERS;
  aiocb &thisblock = this->cbwavblock[ thisindex ];
  if ( aio_read( &thisblock ) == -1 ) {
    this->bodyread = true;
  }
}

/*
## read
Asynchronous read.

Return the number of bytes read. Will read the appropriate amount of bytes for 1 rtp packet for the defined CODEC.
If not ready return -1.

We only support 1 channel. Anything else we need to look at.
*/
bool soundfilereader::read( rawsound &out ) {

  SpinLockGuard guard( this->filelock );

    /* check */
  if ( -1 == this->file ) {
    fprintf( stderr, "No file for open wav sound\n" );
    return false;
  }

  this->parseheader();

  if( this->headerread && this->badheader ) {
    fprintf( stderr, "Bad wav file\n" );
    return false;
  }

  if( !this->headerread ) {
    out.zero();
    return true;
  }

  if( this->initseekmseconds > 0 ) {
    this->realsetposition( this->initseekmseconds );
    out.zero();
    return true;
  }

  auto thisindex = this->currentcbindex % SOUNDFILENUMBUFFERS;
  aiocb &thisblock = this->cbwavblock[ thisindex ];

  if( aio_error( &thisblock ) == EINPROGRESS ) {
    /* return some silence */
    out.zero();
    return true;
  }

  /* success? */
  auto numbytes = aio_return( &thisblock );

  if( -1 == numbytes ) {
    fprintf( stderr, "Bad call to aio_return\n" );
    this->bodyread = true;
    out.zero();
    return true;
  }

  uint8_t *current = ( uint8_t * ) thisblock.aio_buf;
  out = rawsound( current, this->samplecount, this->ploadtype, this->ourwavheader.sample_rate );

  if( static_cast<size_t>( numbytes ) < this->bytecount ) {
    this->bodyread = true;
    /* TODO if we get a partial read we probably should not zero that part */
    out.zero();
    return true;
  }

  /* Get the next block reading */
  auto lastreadoffset = thisblock.aio_offset;
  this->currentcbindex = ( this->currentcbindex + 1 ) % SOUNDFILENUMBUFFERS;

  aiocb &nextblock = this->cbwavblock[ this->currentcbindex ];
  soundbuffer &thisbuffer = this->buffer[ this->currentcbindex ];

  /* it shouldn't reallocate - but just in case */
  thisbuffer.resize( this->bytecount );
  nextblock.aio_buf = thisbuffer.data();

  if( nextblock.aio_offset > this->ourwavheader.chunksize ) {
    nextblock.aio_offset = sizeof( wavheader );
  } else {
    nextblock.aio_offset = lastreadoffset + this->bytecount;
  }

  nextblock.aio_nbytes = this->bytecount;

  /* read next block */
  if ( aio_read( &nextblock ) == -1 ) {
    this->bodyread = true;
    return false;
  }

  return true;
}

/*
# complete
Have we completed reading the file.
*/
bool soundfilereader::complete( void ) {
  SpinLockGuard guard( this->filelock );

  if( this->badheader ) return true;
  if( !this->headerread ) return false;
  if( -1 == this->file ) return true;
  return this->bodyread;
}

/**
 * Cancel all current block reads recalculate offsets and trigger a new read.
 */
void soundfilereader::realsetposition( long mseconds ) {

  while( AIO_NOTCANCELED == aio_cancel( this->file, NULL ) );

  this->bodyread = false;

  this->currentcbindex = 0;
  aiocb &thisblock = this->cbwavblock[ this->currentcbindex ];

  off_t our_aio_offset = ( this->ourwavheader.bit_depth /*16*/ / 8 ) * ( this->ourwavheader.sample_rate / 1000 ) * mseconds; /* bytes per sample */
  our_aio_offset = ( our_aio_offset / this->bytecount ) * this->bytecount; /* realign to the nearest block */
  our_aio_offset += sizeof( wavheader );

  for( auto i = 0; i < SOUNDFILENUMBUFFERS; i++ ) {

    aiocb &nextblock = this->cbwavblock[ i ];
    soundbuffer &thisbuffer = this->buffer[ i ];

    nextblock.aio_offset = our_aio_offset + ( i * this->bytecount );
    thisbuffer.resize( this->bytecount );
    nextblock.aio_buf = thisbuffer.data();
  }

  if ( aio_read( &thisblock ) == -1 ) {
    this->bodyread = true;
  }

  this->initseekmseconds = 0;
}

/*!md
# setposition and getposition
Either queue a set positiuon or perform it depending if our header has been read.
*/
void soundfilereader::setposition( long mseconds ) {

  SpinLockGuard guard( this->filelock );

  /* check */
  if ( -1 == this->file || !this->headerread ) {
    this->initseekmseconds = mseconds;
    return;
  }

  this->realsetposition( mseconds );
}

long soundfilereader::offtomsecs( void ) {
  off_t position = this->cbwavblock[ this->currentcbindex % SOUNDFILENUMBUFFERS ].aio_offset - sizeof( wavheader );
  return position / ( ( this->ourwavheader.bit_depth / 8 ) * ( this->ourwavheader.sample_rate / 1000 ) );
}

long soundfilereader::getposition( void ) {

  SpinLockGuard guard( this->filelock );

  if( this->cbwavblock[ this->currentcbindex % SOUNDFILENUMBUFFERS ].aio_offset <= ( off_t ) sizeof( wavheader ) ) {
    return 0;
  }

  return this->offtomsecs();
}

/* Writer */
/*
# c'stor
Open for writing.
See comments on create regarding mode. This constructor opens for writing.
audio_format = WAVE_FORMAT_PCM etc...
numchannels = 1 | 2
samplerate = 8000 | 16000

Once opened we only accept data in that format and packet size.

NOTE: currently only working for PCM.
*/
soundfilewriter::soundfilewriter( std::string &url, int16_t numchannels, int32_t samplerate ) :
  soundfile( open( url.c_str(), O_WRONLY | O_CREAT | O_TRUNC | O_NONBLOCK, S_IRUSR | S_IWUSR ) ),
  tickcount( 0 ) {

  if ( -1 == this->file ) {
    /* Not much more we can do */
    releasespinlock( this->filelock );
    return;
  }

  initwav( &this->ourwavheader, samplerate );

  /* Now fine tune */
  size_t blocknumbytes = L16NARROWBANDBYTES;
  if( 16000 == samplerate ) {
    blocknumbytes = L16WIDEBANDBYTES;
  }

  if( numchannels != 2 ) {
    numchannels = 1;
  }

  this->ourwavheader.fmt_chunk_size = 16;
  this->ourwavheader.num_channels = numchannels; /* 1 or 2 */
  this->ourwavheader.sample_rate = samplerate;
  this->ourwavheader.byte_rate = samplerate * numchannels * this->ourwavheader.bit_depth / 8;
  this->ourwavheader.chunksize = 0;
  this->ourwavheader.sample_alignment = this->ourwavheader.bit_depth / 8 * numchannels;

  for( auto i = 0; i < SOUNDFILENUMBUFFERS; i++ ) {
    this->cbwavblock[ i ].aio_nbytes = blocknumbytes * numchannels;
  }

  /* write */
  if ( aio_write( &this->cbwavheader ) == -1 ) {
    /* report error somehow. */
    std::cerr << "soundfile unable to write wav header to file " << url << std::endl;
    close( this->file );
    this->file = -1;
    releasespinlock( this->filelock );
    return;
  }

  releasespinlock( this->filelock );
	return;
}

/*
# d-stor
Clean up.
*/
soundfilewriter::~soundfilewriter() {
}

/*
# create
Shared pointer for writing.
*/
soundfilewriter::pointer soundfilewriter::create( std::string &url, int16_t numchannels, int32_t samplerate ) {
  return pointer( new soundfilewriter( url, numchannels, samplerate ) );
}

/*
# write
2 channel write
in = where we get our data
out = where we get our data

This should be called on our tick - 20mS should be ample to complete an async write.
We maintain SOUNDFILENUMBUFFERS to ensure previous writes have an oppertunity to write.
*/
bool soundfilewriter::write( codecx &in, codecx &out ) {

  SpinLockGuard guard( this->filelock );

  if( -1 == this->file ) {
    fprintf( stderr, "soundfile calling write with no open file!\n" );
    return false;
  }

  int16_t *inbuf = nullptr;
  int16_t *outbuf = nullptr;
  size_t inbufsize = 0; /* count of int16 vals */
  size_t outbufsize = 0;
  int inbytespersample = 1;
  int outbytespersample = 1;

  auto format = this->getwavformattopt();

  if( in.hasdata() ) {
    rawsound &inref = in.getref( format );
    if( !inref.isdirty() ) {
      inbuf = ( int16_t * ) inref.c_str();
      inbufsize = inref.size();
      inbytespersample = inref.getbytespersample();
    }
  }

  if( out.hasdata() ) {
    rawsound &outref = out.getref( format );
    if( !outref.isdirty() ) {
      outbuf = ( int16_t * ) outref.c_str();
      outbufsize = outref.size();
      outbytespersample = outref.getbytespersample();
    }
  }

  /* nothing to do */
  if( nullptr == inbuf && nullptr == outbuf ) {
    return false;
  }

  if( inbytespersample != 2 ) inbytespersample = 1;
  if( outbytespersample != 2 ) outbytespersample = 1;

  auto thisindex = this->currentcbindex % SOUNDFILENUMBUFFERS;
  aiocb &thisblock = this->cbwavblock[ thisindex ];
  soundbuffer &thisbuffer = this->buffer[ thisindex ];

  inbufsize = std::min( inbufsize, AIOCBSINGLECHANBUFFNUMSAMPLES );
  outbufsize = std::min( outbufsize, AIOCBSINGLECHANBUFFNUMSAMPLES );

  auto largestbufsize = std::max( inbufsize, outbufsize );
  if( 0 == largestbufsize) {
    return false;
  }

  if( aio_error( &thisblock ) == EINPROGRESS ) {
    fprintf( stderr, "soundfile trying to write a packet whilst last is still in progress\n" );
    return false;
  }

  auto numchannels = this->ourwavheader.num_channels;
  if( numchannels != 2 ) numchannels = 1;

  thisbuffer.resize( largestbufsize * numchannels );
  std::fill_n( thisbuffer.begin(), thisbuffer.size(), 0 );
  auto buf = thisbuffer.data();
  thisblock.aio_buf = buf;

  thisblock.aio_nbytes = largestbufsize * inbytespersample * numchannels;
  thisblock.aio_offset = sizeof( wavheader ) +
            ( this->tickcount * thisblock.aio_nbytes );

  if( inbufsize > 0 && nullptr != inbuf ) {
    for (size_t i = 0; i < inbufsize; i++) {
      *buf = *inbuf;
      inbuf++;
      buf += numchannels;
    }
  }

  if( outbufsize > 0 && nullptr != outbuf ) {
    /* start at the beggining again */
    buf = thisbuffer.data();
    /* only works up to 2 channels - which is all we support */
    if( numchannels == 2 ) buf++;

    for (size_t i = 0; i < outbufsize; i++) {
      auto outval = *outbuf; /////////////////// this is our crash
      *buf += outval;
      outbuf++;
      buf += numchannels;
    }
  }

  if ( aio_write( &thisblock ) == -1 ) {
    fprintf( stderr, "soundfile unable to write wav block to file %s\n", this->url.c_str() );
    return false;
  }

  uint32_t maxbasedonthischunk = 0;
  maxbasedonthischunk = thisblock.aio_offset + thisblock.aio_nbytes;
  if( maxbasedonthischunk > this->ourwavheader.subchunksize ) {
    this->ourwavheader.subchunksize = maxbasedonthischunk;
    this->ourwavheader.chunksize = maxbasedonthischunk + 36;

    /* Update the wav header with size */
    if( aio_error( &this->cbwavheader ) != EINPROGRESS ) { /* silent fail - we will get it on the next one */
      if ( aio_write( &this->cbwavheader ) == -1 ) {
        fprintf( stderr, "soundfile unable to update wav header to file %s\n", this->url.c_str() );
      }
    }
  }

  this->currentcbindex = ( this->currentcbindex + 1 ) % SOUNDFILENUMBUFFERS;
  this->tickcount++;
  return true;
}

/*!md
## initwav
Configure header for basic usage
*/
void initwav( wavheader *w, int samplerate )
{
  w->riff_header[ 0 ] = 'R';
  w->riff_header[ 1 ] = 'I';
  w->riff_header[ 2 ] = 'F';
  w->riff_header[ 3 ] = 'F';

  w->wave_header[ 0 ] = 'W';
  w->wave_header[ 1 ] = 'A';
  w->wave_header[ 2 ] = 'V';
  w->wave_header[ 3 ] = 'E';

  w->fmt_header[ 0 ] = 'f';
  w->fmt_header[ 1 ] = 'm';
  w->fmt_header[ 2 ] = 't';
  w->fmt_header[ 3 ] = ' ';

  w->data_header[ 0 ] = 'd';
  w->data_header[ 1 ] = 'a';
  w->data_header[ 2 ] = 't';
  w->data_header[ 3 ] = 'a';

  w->audio_format = WAVE_FORMAT_PCM;
  w->num_channels = 1;
  w->sample_rate = samplerate;
  w->sample_alignment = 2;
  w->byte_rate = w->sample_rate * 2;
  w->bit_depth = 16;
  w->fmt_chunk_size = 16;
  w->chunksize = 0;
  w->subchunksize = 0;

}

#ifdef NODE_MODULE

/*!md
## wavinfo
For test purposes only. Read and display the wav file header info.
*/
static napi_value wavinfo( napi_env env, napi_callback_info info ) {
//const char *file
  size_t argc = 1;
  napi_value argv[ 1 ];

  char file[ 256 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( 1 != argc ) {
    napi_throw_type_error( env, "0", "Filename?" );
    return NULL;
  }

  size_t copiedout;
  if( napi_ok != napi_get_value_string_utf8( env, argv[ 0 ], file, sizeof( file ), &copiedout ) ) {
    return NULL;
  }

  napi_value returnval = NULL;
  wavheader hd;
  int fd = open( file, O_RDONLY, 0 );
  if( -1 == fd ) {
    napi_throw_type_error( env, "0", "Couldn't open file" );
    return NULL;
  }

  if( sizeof( wavheader ) != read( fd, &hd, sizeof( wavheader ) ) ) {
    napi_throw_type_error( env, "0", "Bad wav header" );
    goto done;
  }

  if( 'R' != hd.riff_header[ 0 ] ||
      'I' != hd.riff_header[ 1 ] ||
      'F' != hd.riff_header[ 2 ] ||
      'F' != hd.riff_header[ 3 ] ) {
    napi_throw_type_error( env, "0", "Bad RIFF" );
    goto done;
  }

  if( 'W' != hd.wave_header[ 0 ] ||
      'A' != hd.wave_header[ 1 ] ||
      'V' != hd.wave_header[ 2 ] ||
      'E' != hd.wave_header[ 3 ] ) {
    napi_throw_type_error( env, "0", "Bad WAVE" );
    goto done;
  }

  if( 'f' != hd.fmt_header[ 0 ] ||
      'm' != hd.fmt_header[ 1 ] ||
      't' != hd.fmt_header[ 2 ] ||
      ' ' != hd.fmt_header[ 3 ] ) {
    napi_throw_type_error( env, "0", "Bad fmt" );
    goto done;
  }

  if( 'd' != hd.data_header[ 0 ] ||
      'a' != hd.data_header[ 1 ] ||
      't' != hd.data_header[ 2 ] ||
      'a' != hd.data_header[ 3 ] ) {
    napi_throw_type_error( env, "0", "Bad data" );
    goto done;
  }

  if( napi_ok != napi_create_object( env, &returnval ) ) return NULL;

  napi_value val;
  if( napi_ok != napi_create_double( env, hd.audio_format, &val ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, returnval, "audioformat", val ) ) return NULL;
  if( napi_ok != napi_create_double( env, hd.num_channels, &val ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, returnval, "channelcount", val ) ) return NULL;
  if( napi_ok != napi_create_double( env, hd.sample_rate, &val ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, returnval, "samplerate", val ) ) return NULL;
  if( napi_ok != napi_create_double( env, hd.sample_alignment, &val ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, returnval, "samplealignment", val ) ) return NULL;
  if( napi_ok != napi_create_double( env, hd.byte_rate, &val ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, returnval, "byterate", val ) ) return NULL;
  if( napi_ok != napi_create_double( env, hd.bit_depth, &val ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, returnval, "bitdepth", val ) ) return NULL;
  if( napi_ok != napi_create_double( env, hd.chunksize, &val ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, returnval, "chunksize", val ) ) return NULL;
  if( napi_ok != napi_create_double( env, hd.fmt_chunk_size, &val ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, returnval, "fmtchunksize", val ) ) return NULL;
  if( napi_ok != napi_create_double( env, hd.subchunksize, &val ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, returnval, "subchunksize", val ) ) return NULL;

  /*std::cout << "Audio format: " << hd.audio_format << std::endl;
  std::cout << "Channel count: " << hd.num_channels << std::endl;
  std::cout << "Sample rate: " << hd.sample_rate << std::endl;
  std::cout << "Sample alignment: " << hd.sample_alignment << std::endl;
  std::cout << "Byte rate: " << hd.byte_rate << std::endl;
  std::cout << "Bit depth: " << hd.bit_depth << std::endl;
  std::cout << "Chunk size: " << hd.chunksize << std::endl;
  std::cout << "Subchunk1Size (should be 16 for PCM): " << hd.fmt_chunk_size << std::endl;
  std::cout << "Subchunk2Size: " << hd.subchunksize << std::endl;*/

done:
  close( fd );
  return returnval;
}

void initrtpsoundfile( napi_env env, napi_value &result ) {
  napi_value soundfile;
  napi_value info;

  if( napi_ok != napi_create_object( env, &soundfile ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "soundfile", soundfile ) ) return;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, wavinfo, nullptr, &info ) ) return;
  if( napi_ok != napi_set_named_property( env, soundfile, "info", info ) ) return;
}

#endif /* NODE_MODULE */
