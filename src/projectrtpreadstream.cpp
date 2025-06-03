projectrtpreadstream::projectrtpreadstream( napi_env env, projectrtpchannel* channel, const std::string& encoding, bool combined )
  : env_( env ), js_this_( nullptr ), on_data_cb_( nullptr ), on_end_cb_( nullptr ),
    tsfn_( nullptr ), combined( combined ), encoding_( encoding ), channel_( channel ) {}

projectrtpreadstream::~projectrtpreadstream() {
  if ( on_data_cb_ ) napi_delete_reference( env_, on_data_cb_ );
  if ( on_end_cb_ ) napi_delete_reference( env_, on_end_cb_ );
  if ( js_this_ ) napi_delete_reference( env_, js_this_ );
  if ( tsfn_ ) napi_release_threadsafe_function( tsfn_, napi_tsfn_abort );
}

void projectrtpreadstream::emitdata( const std::vector<uint8_t>& data ) {
  if ( !tsfn_ ) return;
  auto* vec = new std::vector<uint8_t>( data );
  napi_call_threadsafe_function( tsfn_, vec, napi_tsfn_nonblocking );
}

void projectrtpreadstream::set_data_callback( napi_ref cb ) {
  if ( on_data_cb_ ) napi_delete_reference( env_, on_data_cb_ );
  on_data_cb_ = cb;
}

void projectrtpreadstream::set_end_callback( napi_ref cb ) {
  if ( on_end_cb_ ) napi_delete_reference( env_, on_end_cb_ );
  on_end_cb_ = cb;
}

napi_value projectrtpreadstream::init_js_object() {
  napi_value obj, on_fn, emit_fn, resource_name;

  napi_create_object( env_, &obj );
  napi_create_reference( env_, obj, 1, &js_this_ );

  napi_create_string_utf8( env_, "ondata_callback", NAPI_AUTO_LENGTH, &resource_name );

  napi_create_threadsafe_function(
    env_,
    nullptr,
    nullptr,
    resource_name,
    0,
    1,
    nullptr,
    nullptr,
    this,
    []( napi_env env, napi_value js_cb, void* context, void* raw_data ) {
      auto* vec = static_cast<std::vector<uint8_t>*>( raw_data );
      auto* stream = static_cast<projectrtpreadstream*>( context );

      if ( !vec || !stream || !stream->on_data_cb_ ) {
        delete vec;
        return;
      }

      napi_value js_this, js_callback, js_data;
      napi_get_reference_value( env, stream->js_this_, &js_this );
      napi_get_reference_value( env, stream->on_data_cb_, &js_callback );

      if ( stream->encoding_ == "utf8" ) {
        std::string str( vec->begin(), vec->end() );
        napi_create_string_utf8( env, str.c_str(), str.size(), &js_data );
      } else {
        napi_create_buffer_copy( env, vec->size(), vec->data(), nullptr, &js_data );
      }

      napi_value args[1] = { js_data };
      napi_call_function( env, js_this, js_callback, 1, args, nullptr );
      delete vec;
    },
    &tsfn_
  );

  napi_wrap( env_, obj, this,
    []( napi_env env, void* data, void* ) {
      delete static_cast<projectrtpreadstream*>( data );
    },
    nullptr, nullptr
  );

  napi_create_function( env_, "on", NAPI_AUTO_LENGTH,
    []( napi_env env, napi_callback_info info ) -> napi_value {
      napi_value args[2], this_arg;
      size_t argc = 2;
      napi_get_cb_info( env, info, &argc, args, &this_arg, nullptr );

      char event[16];
      size_t len;
      napi_get_value_string_utf8( env, args[0], event, sizeof( event ), &len );

      projectrtpreadstream* stream;
      napi_unwrap( env, this_arg, ( void** )&stream );

      napi_ref cb;
      napi_create_reference( env, args[1], 1, &cb );

      if ( strcmp( event, "data" ) == 0 ) {
        stream->set_data_callback( cb );
      } else if ( strcmp( event, "end" ) == 0 ) {
        stream->set_end_callback( cb );
      }

      return nullptr;
    }, nullptr, &on_fn
  );
  napi_set_named_property( env_, obj, "on", on_fn );

  napi_create_function( env_, "emitdata", NAPI_AUTO_LENGTH,
    []( napi_env env, napi_callback_info info ) -> napi_value {
      size_t argc = 1;
      napi_value args[1], this_arg;
      void* data;
      napi_get_cb_info( env, info, &argc, args, &this_arg, nullptr );
      napi_unwrap( env, this_arg, &data );

      auto* stream = static_cast<projectrtpreadstream*>( data );
      if ( !stream ) return nullptr;

      bool is_typedarray;
      napi_is_typedarray( env, args[0], &is_typedarray );
      if ( !is_typedarray ) return nullptr;

      napi_typedarray_type type;
      size_t length;
      void* buffer_data;
      napi_value array_buffer;
      size_t byte_offset;

      napi_get_typedarray_info( env, args[0], &type, &length, &buffer_data, &array_buffer, &byte_offset );
      std::vector<uint8_t> vec( ( uint8_t* )buffer_data, ( uint8_t* )buffer_data + length );

      if ( stream->channel_ ) {
        stream->channel_->emittostream( vec );
      }

      return nullptr;
    }, nullptr, &emit_fn
  );
  napi_set_named_property( env_, obj, "emitdata", emit_fn );

  return obj;
}
