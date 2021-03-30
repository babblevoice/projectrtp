
#include <iostream>

#include "projectrtpsoundfile.h"
#include "globals.h"



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
soundfile::soundfile( std::string &url ) :
  file( -1 ),
  url( url ),
  readbuffer( nullptr ),
  currentreadindex( 0 ),
  opened (false ),
  badheader( false ),
  newposition( -1 ),
  headerread( false ),
  writebuffer( nullptr ),
  currentwriteindex( 0 ),
  tickcount( 0 )
{
  int mode = O_RDONLY;
  std::string filenfullpath = mediachroot + url;

  this->file = open( filenfullpath.c_str(), mode | O_NONBLOCK, 0 );
  if ( -1 == this->file )
  {
    /* Not much more we can do */
    return;
  }

  /*
    Soundfile blindly reads the format and passes to the codec - so it must be in a format we support - or there will be silence.
    Our macro player (to be written) will choose the most appropriate file to play based on the codec of the channel.
  */
  this->readbuffer = new uint8_t[ L16WIDEBANDBYTES * SOUNDFILENUMBUFFERS ];

  /* As it is asynchronous - we read wav header + ahead */
  memset( &this->cbwavheader, 0, sizeof( aiocb ) );
  this->cbwavheader.aio_nbytes = sizeof( wavheader );
  this->cbwavheader.aio_fildes = file;
  this->cbwavheader.aio_offset = 0;
  this->cbwavheader.aio_buf = &this->ourwavheader;

  for( auto i = 0; i < SOUNDFILENUMBUFFERS; i++ )
  {
    memset( &this->cbwavblock[ i ], 0, sizeof( aiocb ) );
    this->cbwavblock[ i ].aio_nbytes = L16WIDEBANDBYTES;
    this->cbwavblock[ i ].aio_fildes = this->file;
    this->cbwavblock[ i ].aio_offset = sizeof( wavheader );
    this->cbwavblock[ i ].aio_buf = this->readbuffer + ( i * L16WIDEBANDBYTES );
  }

  /* read */
  if ( aio_read( &this->cbwavheader ) == -1 )
  {
    std::cerr << "aio_read read of header failed in soundfile" << std::endl;
    close( this->file );
    this->file = -1;
    return;
  }

  if ( aio_read( &this->cbwavblock[ this->currentreadindex ] ) == -1 )
  {
    std::cerr << "aio_read read of block failed in soundfile" << std::endl;
    close( this->file );
    this->file = -1;
    return;
  }

	return;

}

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
soundfile::soundfile( std::string &url, uint16_t audio_format, int16_t numchannels, int32_t samplerate ) :
  file( -1 ),
  url( url ),
  readbuffer( nullptr ),
  currentreadindex( 0 ),
  opened (false ),
  badheader( false ),
  newposition( -1 ),
  headerread( false ),
  writebuffer( nullptr ),
  currentwriteindex( 0 ),
  tickcount( 0 )
{
  int mode = O_WRONLY | O_CREAT | O_TRUNC;
  int perms = S_IRUSR | S_IWUSR;
  std::string filenfullpath = mediachroot + url;

  this->file = open( filenfullpath.c_str(), mode | O_NONBLOCK, perms );
  if ( -1 == this->file )
  {
    /* Not much more we can do */
    return;
  }

  initwav( &this->ourwavheader, samplerate );

  /* Now fine tune */
  size_t blocknumbytes = L16NARROWBANDBYTES;
  switch( audio_format )
  {
    case WAVE_FORMAT_PCM:
      this->ourwavheader.bit_depth = 16;
      if( 16000 == samplerate )
      {
        blocknumbytes = L16WIDEBANDBYTES;
      }
      break;
    case WAVE_FORMAT_ALAW:
    case WAVE_FORMAT_MULAW:
      this->ourwavheader.bit_depth = 8;
      blocknumbytes = G711PAYLOADBYTES;
      break;
    case WAVE_FORMAT_POLYCOM_G722:
      this->ourwavheader.bit_depth = 8;
      blocknumbytes = G722PAYLOADBYTES;
      break;
    case WAVE_FORMAT_GLOBAL_IP_ILBC:
      this->ourwavheader.bit_depth = 8;
      blocknumbytes = ILBC20PAYLOADBYTES;
      break;
  }

  this->ourwavheader.audio_format = audio_format;
  this->ourwavheader.fmt_chunk_size = 16;
  this->ourwavheader.num_channels = numchannels; /* or 2 */
  this->ourwavheader.sample_rate = samplerate;
  this->ourwavheader.byte_rate = samplerate * numchannels * this->ourwavheader.bit_depth / 8;
  this->ourwavheader.chunksize = 0;
  this->ourwavheader.sample_alignment = this->ourwavheader.bit_depth / 8 * numchannels;

  /* we need to set chunksize on close? */
  /*
    Soundfile blindly reads the format and passes to the codec - so it must be in a format we support - or there will be silence.

    Our macro player (to be written) will choose the most appropriate file to play based on the codec of the channel.
  */
  this->writebuffer = new uint8_t[ blocknumbytes * numchannels * SOUNDFILENUMBUFFERS ];

  /* As it is asynchronous - we write wav header without waiting - maintain memory */
  memset( &this->cbwavheader, 0, sizeof( aiocb ) );
  this->cbwavheader.aio_nbytes = sizeof( wavheader );
  this->cbwavheader.aio_fildes = this->file;
  this->cbwavheader.aio_offset = 0;
  this->cbwavheader.aio_buf = &this->ourwavheader;

  for( auto i = 0; i < SOUNDFILENUMBUFFERS; i++ )
  {
    memset( &this->cbwavblock[ i ], 0, sizeof( aiocb ) );
    this->cbwavblock[ i ].aio_nbytes = blocknumbytes * numchannels;
    this->cbwavblock[ i ].aio_fildes = this->file;
    this->cbwavblock[ i ].aio_offset = sizeof( wavheader );
    this->cbwavblock[ i ].aio_buf = this->writebuffer + ( i * blocknumbytes );
  }

  /* write */
  if ( aio_write( &this->cbwavheader ) == -1 )
  {
    /* report error somehow. */
    std::cerr << "soundfile unable to write wav header to file " << url << std::endl;
    close( this->file );
    this->file = -1;
    return;
  }

	return;
}

/*
# write
2 channel write
in = where we get our data
out = where we get our data

This should be called on our tick - 20mS should be ample to complete an async write.
We maintain SOUNDFILENUMBUFFERS to ensure previous writes have an oppertunity to write.
*/
bool soundfile::write( codecx &in, codecx &out )
{
  int16_t *inbuf = nullptr;
  int16_t *outbuf = nullptr;
  size_t bufsize = 0;
  int bytespersample = 1;

  if( in.hasdata() )
  {
    rawsound &inref = in.getref( this->getwavformattopt() );
    if( !inref.isdirty() )
    {
      inbuf = ( int16_t * ) inref.c_str();
      bufsize = inref.size();
      bytespersample = inref.getbytespersample();
    }
  }

  if( out.hasdata() )
  {
    rawsound &outref = out.getref( this->getwavformattopt() );
    if( !outref.isdirty() )
    {
      outbuf = ( int16_t * ) outref.c_str();
      bufsize = outref.size();
      bytespersample = outref.getbytespersample();
    }
  }

  if( ( nullptr == inbuf && nullptr == outbuf ) || 0 == bufsize )
  {
    return false;
  }

  if( aio_error( &this->cbwavblock[ this->currentwriteindex ] ) == EINPROGRESS )
  {
    std::cerr << "soundfile trying to write a packet whilst last is still in progress" << std::endl;
    return false;
  }

  if( nullptr == this->writebuffer )
  {
    std::cerr << "soundfile no write buffer!" << std::endl;
    return false;
  }

  this->cbwavblock[ this->currentwriteindex ].aio_nbytes = bufsize * bytespersample * this->ourwavheader.num_channels;
  this->cbwavblock[ this->currentwriteindex ].aio_offset = sizeof( wavheader ) +
            ( this->tickcount * this->cbwavblock[ this->currentwriteindex ].aio_nbytes );

  int16_t *buf = ( int16_t * ) this->cbwavblock[ this->currentwriteindex ].aio_buf;
  memset( buf, 0, this->cbwavblock[ this->currentwriteindex ].aio_nbytes );

  if( nullptr != inbuf )
  {
    for( size_t i = 0; i < bufsize; i++ )
    {
      *buf = *inbuf;
      inbuf++;
      buf += this->ourwavheader.num_channels;
    }
  }

  buf = ( int16_t * ) this->cbwavblock[ this->currentwriteindex ].aio_buf;
  if( this->ourwavheader.num_channels > 1 ) /* only works up to 2 channels - which is all we support */
  {
    buf++;
  }

  if( nullptr != outbuf )
  {
    for( size_t i = 0; i < bufsize; i++ )
    {
      *buf += *outbuf;
      outbuf ++;
      buf += this->ourwavheader.num_channels;
    }
  }

  if ( aio_write( &this->cbwavblock[ this->currentwriteindex ] ) == -1 )
  {
    std::cerr << "soundfile unable to write wav block to file " << this->url << std::endl;
    return false;
  }

  uint32_t maxbasedonthischunk = 0;
  maxbasedonthischunk = this->cbwavblock[ this->currentwriteindex ].aio_offset + this->cbwavblock[ this->currentwriteindex ].aio_nbytes;
  if( maxbasedonthischunk > this->ourwavheader.subchunksize )
  {
    this->ourwavheader.subchunksize = maxbasedonthischunk;
    this->ourwavheader.chunksize = maxbasedonthischunk + 36;

    /* Update the wav header with size */
    if ( aio_write( &this->cbwavheader ) == -1 )
    {
      std::cerr << "soundfile unable to update wav header to file " << this->url << std::endl;
    }
  }

  this->currentwriteindex = ( this->currentwriteindex + 1 ) % SOUNDFILENUMBUFFERS;
  this->tickcount++;
  return true;
}

uint32_t soundfile::getwriteduration( void ) /* mS */
{
  return this->ourwavheader.subchunksize / this->ourwavheader.byte_rate;
}

/*!md
# d-stor
Clean up.
*/
soundfile::~soundfile()
{
  if( nullptr != this->readbuffer )
  {
    delete[] this->readbuffer;
  }

  if( nullptr != this->writebuffer )
  {
    delete[] this->writebuffer;
  }

  if ( -1 != this->file )
  {
    close( this->file );
  }
}

/*
# create
Shared pointer version of us.
*/
soundfile::pointer soundfile::create( std::string &url )
{
  return pointer( new soundfile( url ) );
}

/*
# create
Shared pointer for writing.
*/
soundfile::pointer soundfile::create( std::string &url, uint16_t audio_format, int16_t numchannels, int32_t samplerate )
{
  return pointer( new soundfile( url, audio_format, numchannels, samplerate ) );
}

/*
## read
Asynchronous read.

Return the number of bytes read. Will read the appropriate amount of bytes for 1 rtp packet for the defined CODEC.
If not ready return -1.

We only support 1 channel. Anything else we need to look at.
*/
bool soundfile::read( rawsound &out )
{
  /* check */
  if ( -1 == this->file )
  {
    return false;
  }

  if( false == this->headerread &&
      aio_error( &this->cbwavheader ) == EINPROGRESS )
  {
    std::cerr << "Read of soundfile wav header has not completed" << std::endl;
    return false;
  }

  this->headerread = true;

  if( aio_error( &this->cbwavblock[ this->currentreadindex ] ) == EINPROGRESS )
  {
    std::cerr << "Read of soundfile wav block has not completed" << std::endl;
    return false;
  }

  /* success? */
  int numbytes = aio_return( &this->cbwavblock[ this->currentreadindex ] );

  if( -1 == numbytes || 0 == numbytes )
  {
    return false;
  }

  this->opened = true;
  if( 'W' != this->ourwavheader.wave_header[ 0 ] )
  {
    this->badheader = true;
  }
  this->badheader = false;

  switch( this->ourwavheader.sample_rate )
  {
    case 8000:
    case 16000:
      break;
    default:
      return false;
  }

  int ploadtype = L168KPAYLOADTYPE;
  int blocksize = L16NARROWBANDBYTES;
  switch( this->ourwavheader.audio_format )
  {
    case WAVE_FORMAT_PCM:
    {
      if( 8000 == this->ourwavheader.sample_rate )
      {
        ploadtype = L168KPAYLOADTYPE;
        blocksize = L16NARROWBANDBYTES;
      }
      else if( 16000 == this->ourwavheader.sample_rate )
      {
        ploadtype = L1616KPAYLOADTYPE;
        blocksize = L16WIDEBANDBYTES;
      }
      else
      {
        return false;
      }
      break;
    }
    case WAVE_FORMAT_ALAW:
    {
      ploadtype = PCMAPAYLOADTYPE;
      blocksize = G711PAYLOADBYTES;
      break;
    }
    case WAVE_FORMAT_MULAW:
    {
      ploadtype = PCMUPAYLOADTYPE;
      blocksize = G711PAYLOADBYTES;
      break;
    }
    case WAVE_FORMAT_POLYCOM_G722:
    {
      ploadtype = G722PAYLOADTYPE;
      blocksize = G722PAYLOADBYTES;
      break;
    }
    case WAVE_FORMAT_GLOBAL_IP_ILBC:
    {
      ploadtype = ILBCPAYLOADTYPE;
      blocksize = ILBC20PAYLOADBYTES;
      break;
    }
    default:
    {
      return false;
    }
  }

  uint8_t *current = ( uint8_t * ) this->cbwavblock[ this->currentreadindex ].aio_buf;
  out = rawsound( current, blocksize, ploadtype, this->ourwavheader.sample_rate );

  /* Get the next block reading */
  auto lastreadoffset = this->cbwavblock[ this->currentreadindex ].aio_offset;
  this->currentreadindex = ( this->currentreadindex + 1 ) % SOUNDFILENUMBUFFERS;

  if( -1 == this->newposition )
  {
    this->cbwavblock[ this->currentreadindex ].aio_offset = lastreadoffset + blocksize;
  }
  else
  {
    this->cbwavblock[ this->currentreadindex ].aio_offset = ( this->ourwavheader.bit_depth / 8 ) * ( this->ourwavheader.sample_rate / 1000 ) * this->newposition; /* bytes per sample */
    this->cbwavblock[ this->currentreadindex ].aio_offset = ( this->cbwavblock[ this->currentreadindex ].aio_offset / blocksize ) * blocksize; /* realign to the nearest block */
    this->cbwavblock[ this->currentreadindex ].aio_offset += sizeof( wavheader );
  }

  this->cbwavblock[ this->currentreadindex ].aio_nbytes = blocksize;

  if( this->cbwavblock[ this->currentreadindex ].aio_offset > ( __off_t ) ( this->ourwavheader.chunksize + sizeof( wavheader ) ) )
  {
    this->cbwavblock[ this->currentreadindex ].aio_offset = sizeof( wavheader );
  }

  /* read next block */
  if ( aio_read( &this->cbwavblock[ this->currentreadindex ] ) == -1 )
  {
    close( this->file );
    this->file = -1;
    return false;
  }

  if( -1 != this->newposition )
  {
    this->newposition = -1;
    return false;
  }

  return true;
}

/*!md
# setposition and getposition
Gets and sets the position in terms of mS.
*/
void soundfile::setposition( long mseconds )
{
  this->newposition = mseconds;
}

long soundfile::offtomsecs( void )
{
  __off_t position = this->cbwavblock[ this->currentreadindex ].aio_offset - sizeof( wavheader );
  return position / ( ( this->ourwavheader.bit_depth / 8 ) * ( this->ourwavheader.sample_rate / 1000 ) );
}

long soundfile::getposition( void )
{
  if( this->cbwavblock[ this->currentreadindex ].aio_offset <= ( __off_t ) sizeof( wavheader ) )
  {
    return 0;
  }

  if( -1 != this->newposition )
  {
    return this->newposition;
  }

  return this->offtomsecs();
}

uint8_t soundfile::getwavformattopt( void )
{
  switch( this->ourwavheader.audio_format )
  {
    case WAVE_FORMAT_PCM:
    {
      if( 8000 == this->ourwavheader.sample_rate )
      {
        return L168KPAYLOADTYPE;
      }
      return L1616KPAYLOADTYPE;
    }
    case WAVE_FORMAT_ALAW:
      return PCMAPAYLOADTYPE;
    case WAVE_FORMAT_MULAW:
      return PCMUPAYLOADTYPE;
    case WAVE_FORMAT_POLYCOM_G722:
      return G722PAYLOADTYPE;
    case WAVE_FORMAT_GLOBAL_IP_ILBC:
      return ILBC20PAYLOADBYTES;
  }
  std::cerr << "soundfile::getwavformattopt unknown wav file format to convert to RTP PT" << std::endl;
  return L1616KPAYLOADTYPE;
}

/*
# wavformatfrompt
Return the best fit format for payload type for recording.
*/
uint16_t soundfile::wavformatfrompt( uint8_t pt )
{
#if 0
  switch( pt )
  {
    case PCMUPAYLOADTYPE:
      return WAVE_FORMAT_MULAW;
    case PCMAPAYLOADTYPE:
      return WAVE_FORMAT_ALAW;
    case ILBCPAYLOADTYPE:
      return WAVE_FORMAT_GLOBAL_IP_ILBC;
    case G722PAYLOADTYPE:
      return WAVE_FORMAT_POLYCOM_G722;
  }
#endif
  return WAVE_FORMAT_PCM;
}

int soundfile::getsampleratefrompt( uint8_t pt )
{
  if( G722PAYLOADTYPE == pt )
  {
    return 16000;
  }
  return 8000;
}

/*!md
# complete
Have we completed reading the file.
*/
bool soundfile::complete( void )
{
  if( false == this->opened )
  {
    return false;
  }
  return this->cbwavblock[ this->currentreadindex ].aio_offset > this->ourwavheader.chunksize;
}


/*!md
## wavinfo
For test purposes only. Read and display the wav file header info.
*/
void wavinfo( const char *file )
{
  wavheader hd;
  int fd = open( file, O_RDONLY, 0 );
  if( -1 == fd )
  {
    std::cerr << "Couldn't open file " << file << std::endl;
    return;
  }

  read( fd, &hd, sizeof( wavheader ) );

  if( 'R' != hd.riff_header[ 0 ] ||
      'I' != hd.riff_header[ 1 ] ||
      'F' != hd.riff_header[ 2 ] ||
      'F' != hd.riff_header[ 3 ] )
  {
    std::cout << "Bad riff" << std::endl;
    goto done;
  }

  if( 'W' != hd.wave_header[ 0 ] ||
      'A' != hd.wave_header[ 1 ] ||
      'V' != hd.wave_header[ 2 ] ||
      'E' != hd.wave_header[ 3 ] )
  {
    std::cout << "Bad wav" << std::endl;
    goto done;
  }

  if( 'f' != hd.fmt_header[ 0 ] ||
      'm' != hd.fmt_header[ 1 ] ||
      't' != hd.fmt_header[ 2 ] ||
      ' ' != hd.fmt_header[ 3 ] )
  {
    std::cout << "Bad format" << std::endl;
    goto done;
  }

  if( 'd' != hd.data_header[ 0 ] ||
      'a' != hd.data_header[ 1 ] ||
      't' != hd.data_header[ 2 ] ||
      'a' != hd.data_header[ 3 ] )
  {
    std::cout << "Bad data header" << std::endl;
    goto done;
  }

  std::cout << "Audio format: " << hd.audio_format << std::endl;
  std::cout << "Channel count: " << hd.num_channels << std::endl;
  std::cout << "Sample rate: " << hd.sample_rate << std::endl;
  std::cout << "Sample alignment: " << hd.sample_alignment << std::endl;
  std::cout << "Byte rate: " << hd.byte_rate << std::endl;
  std::cout << "Bit depth: " << hd.bit_depth << std::endl;
  std::cout << "Chunk size: " << hd.chunksize << std::endl;
  std::cout << "Subchunk1Size (should be 16 for PCM): " << hd.fmt_chunk_size << std::endl;
  std::cout << "Subchunk2Size: " << hd.subchunksize << std::endl;


done:
  close( fd );
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
