
#include "projectrtpbuffer.h"

rtpbuffer::rtpbuffer( int buffercount, int waterlevel ) :
  reserved( nullptr ),
  waterlevel( waterlevel ),
  outsn( 0 ) {

  this->buffer.resize( buffercount );
  this->orderedrtpdata.resize( buffercount );

  for ( int i = 0; i < buffercount; i++ ) {
    this->orderedrtpdata.at( i ) = nullptr;
    this->availablertpdata.push( &this->buffer.at( i ) );
  }
}

rtpbuffer::pointer rtpbuffer::create( int buffercount, int waterlevel ) {
  return pointer( new rtpbuffer( buffercount, waterlevel ) );
}

/*
Returns the next packet in order - does not modify our structure.
*/
rtppacket* rtpbuffer::peek( void ) {
  rtppacket *out = this->orderedrtpdata.at( this->outsn % this->orderedrtpdata.size() );
  if( nullptr == out ) return nullptr;
  if( out->getsequencenumber() != this->outsn ) return nullptr;

  return out;
}

/*
Returns the next packet in order.
*/
rtppacket* rtpbuffer::pop( void ) {
  rtppacket *out = this->peek();
  this->orderedrtpdata.at( this->outsn % this->orderedrtpdata.size() ) = nullptr;
  this->outsn++;
  if( nullptr == out ) return nullptr;

  this->availablertpdata.push( out );
  return out;
}

/*
Stores the last reserved packet.
*/
void rtpbuffer::push( void ) {

  if( nullptr == this->reserved ) {
    return;
  }

  uint16_t sn = this->reserved->getsequencenumber();

  /*
    When this is our first packet entering (either becuase we were emptied or
    this is the first use) then we set the out sn to be the water level ahead to
    allow packets to build up to that level and gain some order.
    -1 as we have a reserved packet
  */
  if( this->availablertpdata.size() == ( this->buffer.size() - 1 ) ) {
    this->outsn = sn - this->buffer.size() + this->waterlevel;
    /* printf( "outsn: %.2X\n", this->outsn ); */
  } else {
    /* out of range - based on our outsn counter */
    if( ( ( uint16_t )( sn - this->outsn ) ) > ( ( uint16_t ) this->buffer.size() ) ) {
      this->availablertpdata.push( this->reserved );
      this->reserved = nullptr;
      return;
    }
  }

  if( nullptr == this->orderedrtpdata.at( sn % this->orderedrtpdata.size() ) ) {
    this->orderedrtpdata.at( sn % this->orderedrtpdata.size() ) = this->reserved;
  }

  this->reserved = nullptr;
}

/*
Returns an available buffer (if there is one free). This is then passed into
push once we have received data for it.
*/
rtppacket* rtpbuffer::reserve( void ) {
  if( nullptr != this->reserved ) return this->reserved;
  if( 0 == this->availablertpdata.size() ) return nullptr;
  this->reserved = this->availablertpdata.front();
  this->availablertpdata.pop();
  return this->reserved;
}


#ifdef NODE_MODULE

// uuidgen | sed -r -e 's/-//g' -e 's/(.{16})(.*)/0x\1, 0x\2/'
static const napi_type_tag buffercreatetag = {
  0xcc725991e3ee4b67, 0xafabe716c9483c5c
};


static rtpbuffer *getrtpbufferfromthis( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];
  napi_value thisarg;
  bool isrtpbuffer;

  rtpbuffer *pb;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, &thisarg, nullptr ) ) return nullptr;
  if( napi_ok != napi_check_object_type_tag( env, thisarg, &buffercreatetag, &isrtpbuffer ) ) return nullptr;

  if( !isrtpbuffer ) {
    napi_throw_type_error( env, "0", "Not an RTP Buffer type" );
    return nullptr;
  }

  if( napi_ok != napi_unwrap( env, thisarg, ( void** ) &pb ) ) {
    napi_throw_type_error( env, "1", "Buffer didn't unwrap" );
    return nullptr;
  }

  return pb;
}


static napi_value bufferpop( napi_env env, napi_callback_info info ) {

  napi_value result;
  void *outdata;

  rtpbuffer *pb = getrtpbufferfromthis( env, info );
  if( nullptr == pb ) return NULL;


  rtppacket *p = pb->pop();
  if( nullptr == p ) {
    return NULL;
  }

  if( napi_ok != napi_create_buffer_copy( env, p->length, ( const void* ) p->pk, &outdata, &result ) ) {
    return NULL;
  }

  return result;
}

static napi_value bufferpeek( napi_env env, napi_callback_info info ) {

  napi_value result;
  void *outdata;

  rtpbuffer *pb = getrtpbufferfromthis( env, info );
  if( nullptr == pb ) return NULL;


  rtppacket *p = pb->peek();
  if( nullptr == p ) {
    return NULL;
  }

  if( napi_ok != napi_create_buffer_copy( env, p->length, ( const void* ) p->pk, &outdata, &result ) ) {
    return NULL;
  }

  return result;
}

/**

*/
static napi_value bufferpush( napi_env env, napi_callback_info info ) {

  size_t argc = 1;
  napi_value argv[ 1 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  bool haspayload = false;
  if( napi_ok != napi_has_named_property( env, argv[ 0 ], "payload", &haspayload ) ) {
    napi_throw_error( env, "0", "NAPI Error" );
    return NULL;
  }

  napi_value payload;
  if( napi_ok != napi_get_named_property( env, argv[ 0 ], "payload", &payload ) ) {
    napi_throw_error( env, "1", "NAPI Error" );
    return NULL;
  }

  bool isbuffer = false;
  if( napi_ok != napi_is_buffer( env, payload, &
    isbuffer ) ) {
    napi_throw_error( env, "2", "NAPI Error" );
    return NULL;
  }

  if( !isbuffer ) {
    napi_throw_type_error( env, "2", "Payload must be a buffer" );
    return NULL;
  }

  size_t bufferlength;
  uint8_t *bufferdata;

  if( napi_ok != napi_get_buffer_info( env, payload, ( void ** ) &bufferdata, &bufferlength ) ) {
    napi_throw_type_error( env, "2", "Couldn't get buffer data" );
    return NULL;
  }

  rtpbuffer *pb = getrtpbufferfromthis( env, info );
  if( nullptr == pb ) return NULL;

  rtppacket *p = pb->reserve();
  if( nullptr == p ) {
    return NULL;
  }

  int buflength = std::min( ( int ) bufferlength, RTPMAXLENGTH );
  for( int i = 0; i < buflength; i++ ) {
    p->pk[ i ] = bufferdata[ i ];
  }

  p->length = buflength;
  //printf( "sn: %.2X\n", p->getsequencenumber() );

  pb->push();

  return NULL;
}

void bufferdestroy( napi_env env, void* instance, void* /* hint */ ) {
  delete ( (rtpbuffer* ) instance );
}

static napi_value buffercreate( napi_env env, napi_callback_info info ) {

  size_t argc = 1;
  napi_value argv[ 1 ];
  napi_value npush, npop, npeek, nsize, nwater;

  int32_t packetcount, packetwaterlevel;

  packetcount = 20;
  packetwaterlevel = 15;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( argc > 0 ) {
    if( napi_ok == napi_get_named_property( env, argv[ 0 ], "size", &nsize ) ) {
      napi_get_value_int32( env, nsize, &packetcount );
    }

    if( napi_ok == napi_get_named_property( env, argv[ 0 ], "waterlevel", &nwater ) ) {
      napi_get_value_int32( env, nwater, &packetwaterlevel );
    }
  }

  napi_value result;
  rtpbuffer *pb = new rtpbuffer( packetcount, packetwaterlevel );

  if( napi_ok != napi_create_object( env, &result ) ) return NULL;
  if( napi_ok != napi_type_tag_object( env, result, &buffercreatetag ) ) return NULL;
  if( napi_ok != napi_wrap( env, result, ( void * )pb, bufferdestroy, nullptr, nullptr ) ) return NULL;

  if ( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, bufferpush, nullptr, &npush ) ) return NULL;
  if ( napi_ok != napi_set_named_property( env, result, "push", npush ) ) return NULL;

  if ( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, bufferpop, nullptr, &npop ) ) return NULL;
  if ( napi_ok != napi_set_named_property( env, result, "pop", npop ) ) return NULL;

  if ( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, bufferpeek, nullptr, &npeek ) ) return NULL;
  if ( napi_ok != napi_set_named_property( env, result, "peek", npeek ) ) return NULL;

  /*
  std::string res = "Hello World!";
  if ( napi_ok != napi_create_string_utf8( env, res.c_str(), res.size(), &result ) ) {
    return NULL;
  }
  */

  return result;
}

void initrtpbuffer( napi_env env, napi_value &result ) {
  napi_value rtpbuff;
  napi_value bcreate;

  if( napi_ok != napi_create_object( env, &rtpbuff ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "rtpbuffer", rtpbuff ) ) return;
  if ( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, buffercreate, nullptr, &bcreate ) ) return;
  if ( napi_ok != napi_set_named_property( env, rtpbuff, "create", bcreate ) ) return;

}

#endif /* NODE_MODULE */
