// Original xirt.us fluid plasma, from the site source, unchanged.

	function getShader(gl, id) {
		var shaderScript = document.getElementById(id);
		var str = "";
		var k = shaderScript.firstChild;
		while (k) {
			if (k.nodeType == 3)
				str += k.textContent;
			k = k.nextSibling;
		}
		
		var fsIncScript = document.getElementById("shader-fs-inc");
		var incStr = "";
		k = fsIncScript.firstChild;
		while (k) {
			if (k.nodeType == 3)
				incStr += k.textContent;
			k = k.nextSibling;
		}
		
		var shader;
		if (shaderScript.type == "x-shader/x-fragment"){
			str = incStr + str;
			shader = gl.createShader(gl.FRAGMENT_SHADER);
		}
		else if (shaderScript.type == "x-shader/x-vertex")
			shader = gl.createShader(gl.VERTEX_SHADER);
		else
			return null;
		gl.shaderSource(shader, str);
		gl.compileShader(shader);
		if (gl.getShaderParameter(shader, gl.COMPILE_STATUS) == 0)
			alert("error compiling shader '"+id+"'\n\n"+gl.getShaderInfoLog(shader));
		return shader;
	}

	requestAnimFrame = (function() {
		return window.webkitRequestAnimationFrame
				|| window.mozRequestAnimationFrame ||
		
	 function(callback, element) {	setTimeout(callback, 1000 / 60);};
	})();

	var gl;
	
	var prog_copy;
	var prog_advance;
	var prog_composite;
	var prog_blur_horizontal;
	var prog_blur_vertical;

	var prog_fluid_init;
	var prog_fluid_add_mouse_motion;
	var prog_fluid_advect;
	var prog_fluid_p;
	var prog_fluid_div;

	var FBO_main;
	var FBO_main2;

	var FBO_noise;
	var FBO_blur;
	var FBO_blur2;
	var FBO_blur3;
	var FBO_blur4;
	var FBO_blur5;
	var FBO_blur6;

	var FBO_helper;
	var FBO_helper2;
	var FBO_helper3;
	var FBO_helper4;
	var FBO_helper5;
	var FBO_helper6;

	var texture_main_l; // main, linear
	var texture_main_n; // main, nearest (accurate uv access on the same buffer)
	var texture_main2_l; // main double buffer, linear
	var texture_main2_n; // main double buffer, nearest (accurate uv access on the same buffer)
	var texture_helper; // needed for multi-pass shader programs (2-pass Gaussian blur)
	var texture_helper2; // (1/4 resolution )
	var texture_helper3; // (1/16 resolution )
	var texture_helper4; // (1/256 resolution )
	var texture_helper5;
	var texture_helper6;

	var texture_blur; // full resolution blur result
	var texture_blur2; // double blur
	var texture_blur3; // quad blur
	var texture_blur4; // use low resolutions wisely ;)
	var texture_blur5;
	var texture_blur6;

	var texture_noise_n; // nearest
	var texture_noise_l; // linear interpolation
	
	// fluid simulation GL textures and frame buffer objects
	
	var texture_fluid_v;	// velocities
	var texture_fluid_p;	// pressure
	var texture_fluid_store;  
	var texture_fluid_backbuffer;

	var FBO_fluid_v;
	var FBO_fluid_p;
	var FBO_fluid_store;
	var FBO_fluid_backbuffer;

	var simScale = 2; // factor for reduced buffer size (TODO) 

	// main animation loop vars

	var sizeX = 1024;	// texture size (must be powers of two)
	var sizeY = 512;
	
	var viewX = sizeX;	// viewport size (ideally exactly the texture size)
	var viewY = sizeY;

	var halted = false;
	var delay = 1/60;
	var it = 1;	// main loop buffer toggle
	var frame = 0; // frame counter (to be resetted every 1000ms)
	var fps;
	var time;
	var timer;
	
	var mouseX = 0.5;
	var mouseY = 0.5;
	var oldMouseX = 0;
	var oldMouseY = 0;
	var mouseDx = 0;
	var mouseDy = 0;


	function load() {
		clearInterval(timer);
		var c = document.getElementById("c");
		try {
			gl = c.getContext("experimental-webgl", {
				depth : false
			});
		} catch (e) {}
		if (!gl) {
			alert("Your browser does not support WebGL");
			return;
		}
		document.onmousemove = function(evt) {
			mouseX = evt.pageX / viewX;
			mouseY = 1 - evt.pageY / viewY;
		};
		document.onclick = function(evt) {
			//stop = 0;//(stop == 1)?0:1;
		};
		
		viewX = window.innerWidth;
		viewY = window.innerHeight;

		c.width = viewX;
		c.height = viewY;

		prog_copy = createAndLinkProgram("shader-fs-copy");
		
		prog_advance = createAndLinkProgram("shader-fs-advance");
		prog_composite = createAndLinkProgram("shader-fs-composite");
		prog_blur_horizontal = createAndLinkProgram("shader-fs-blur-horizontal");
		prog_blur_vertical = createAndLinkProgram("shader-fs-blur-vertical");
		
		prog_fluid_init = createAndLinkProgram("shader-fs-init"); // sets encoded values to zero
		prog_fluid_add_mouse_motion = createAndLinkProgram("shader-fs-add-mouse-motion");
		prog_fluid_advect = createAndLinkProgram("shader-fs-advect");
		prog_fluid_p = createAndLinkProgram("shader-fs-p");
		prog_fluid_div = createAndLinkProgram("shader-fs-div");

		// two triangles ought to be enough for anyone ;)
		var posBuffer = gl.createBuffer();
		gl.bindBuffer(gl.ARRAY_BUFFER, posBuffer);

		var vertices = new Float32Array([ -1, -1, 0, 1, -1, 0, -1, 1, 0, 1, 1, 0 ]);

		var aPosLoc = gl.getAttribLocation(prog_advance, "aPos");
		gl.enableVertexAttribArray(aPosLoc);

		var aTexLoc = gl.getAttribLocation(prog_advance, "aTexCoord");
		gl.enableVertexAttribArray(aTexLoc);

		var texCoords = new Float32Array([ 0, 0, 1, 0, 0, 1, 1, 1 ]);

		var texCoordOffset = vertices.byteLength;

		gl.bufferData(gl.ARRAY_BUFFER, texCoordOffset + texCoords.byteLength, gl.STATIC_DRAW);
		gl.bufferSubData(gl.ARRAY_BUFFER, 0, vertices);
		gl.bufferSubData(gl.ARRAY_BUFFER, texCoordOffset, texCoords);
		gl.vertexAttribPointer(aPosLoc, 3, gl.FLOAT, gl.FALSE, 0, 0);
		gl.vertexAttribPointer(aTexLoc, 2, gl.FLOAT, gl.FALSE, 0, texCoordOffset);

		var noisePixels = [], pixels = [], simpixels = [], pixels2 = [], pixels3 = [], pixels4 = [], pixels5 = [], pixels6 = [];
		for ( var i = 0; i < sizeX; i++) {
			for ( var j = 0; j < sizeY; j++) {
				noisePixels.push(Math.random() * 255, Math.random() * 255, Math.random() * 255, 255);
				pixels.push(0, 0, 0, 255);
				if( i < sizeX/simScale && j < sizeY/simScale) simpixels.push(0, 0, 0, 255);
				if( i < sizeX/2 && j < sizeY/2) pixels2.push(0, 0, 0, 255);
				if( i < sizeX/4 && j < sizeY/4) pixels3.push(0, 0, 0, 255);
				if( i < sizeX/8 && j < sizeY/8) pixels4.push(0, 0, 0, 255);
				if( i < sizeX/16 && j < sizeY/16) pixels5.push(0, 0, 0, 255);
				if( i < sizeX/32 && j < sizeY/32) pixels6.push(0, 0, 0, 255);
			}
		}

		FBO_main = gl.createFramebuffer();
		FBO_main2 = gl.createFramebuffer();
		var glPixels;
		glPixels = new Uint8Array(noisePixels);
		texture_main_n = createAndBindTexture(glPixels, 1, FBO_main, gl.NEAREST);
		texture_main2_n = createAndBindTexture(glPixels, 1, FBO_main2, gl.NEAREST);
		glPixels = new Uint8Array(noisePixels);
		texture_main_l = createAndBindTexture(glPixels, 1, FBO_main, gl.LINEAR);
		texture_main2_l = createAndBindTexture(glPixels, 1, FBO_main2, gl.LINEAR);

		FBO_fluid_p = gl.createFramebuffer();
		FBO_fluid_v = gl.createFramebuffer();
		FBO_fluid_store = gl.createFramebuffer();
		FBO_fluid_backbuffer = gl.createFramebuffer();
		texture_fluid_v = createAndBindSimulationTexture(new Uint8Array(simpixels), FBO_fluid_v);
		texture_fluid_p = createAndBindSimulationTexture(new Uint8Array(simpixels), FBO_fluid_p);
		texture_fluid_store = createAndBindSimulationTexture(new Uint8Array(simpixels), FBO_fluid_store);
		texture_fluid_backbuffer = createAndBindSimulationTexture(new Uint8Array(simpixels), FBO_fluid_backbuffer);
		
		FBO_helper = gl.createFramebuffer();
		FBO_helper2 = gl.createFramebuffer();
		FBO_helper3 = gl.createFramebuffer();
		FBO_helper4 = gl.createFramebuffer();
		FBO_helper5 = gl.createFramebuffer();
		FBO_helper6 = gl.createFramebuffer();
		texture_helper = createAndBindTexture(new Uint8Array(pixels), 1, FBO_helper, gl.NEAREST); // helper buffers for the two-pass Gaussian blur calculation basically
		texture_helper2 = createAndBindTexture(new Uint8Array(pixels2), 2, FBO_helper2, gl.NEAREST);
		texture_helper3 = createAndBindTexture(new Uint8Array(pixels3), 4, FBO_helper3, gl.NEAREST);
		texture_helper4 = createAndBindTexture(new Uint8Array(pixels4), 8, FBO_helper4, gl.NEAREST);
		texture_helper5 = createAndBindTexture(new Uint8Array(pixels5), 16, FBO_helper5, gl.NEAREST);
		texture_helper6 = createAndBindTexture(new Uint8Array(pixels6), 32, FBO_helper6, gl.NEAREST);

		FBO_blur = gl.createFramebuffer();
		FBO_blur2 = gl.createFramebuffer();
		FBO_blur3 = gl.createFramebuffer();
		FBO_blur4 = gl.createFramebuffer();
		FBO_blur5 = gl.createFramebuffer();
		FBO_blur6 = gl.createFramebuffer();
		texture_blur = createAndBindTexture(new Uint8Array(pixels), 1, FBO_blur, gl.LINEAR);
		texture_blur2 = createAndBindTexture(new Uint8Array(pixels2), 2, FBO_blur2, gl.LINEAR);
		texture_blur3 = createAndBindTexture(new Uint8Array(pixels3), 4, FBO_blur3, gl.LINEAR);
		texture_blur4 = createAndBindTexture(new Uint8Array(pixels4), 8, FBO_blur4, gl.LINEAR);
		texture_blur5 = createAndBindTexture(new Uint8Array(pixels5), 16, FBO_blur5, gl.LINEAR);
		texture_blur6 = createAndBindTexture(new Uint8Array(pixels6), 32, FBO_blur6, gl.LINEAR);

		FBO_noise = gl.createFramebuffer();
		glPixels = new Uint8Array(noisePixels);
		texture_noise_n = createAndBindTexture(glPixels, 1, FBO_noise, gl.NEAREST);
		texture_noise_l = createAndBindTexture(glPixels, 1, FBO_noise, gl.LINEAR);

		gl.activeTexture(gl.TEXTURE2); gl.bindTexture(gl.TEXTURE_2D, texture_blur);
		gl.activeTexture(gl.TEXTURE3); gl.bindTexture(gl.TEXTURE_2D, texture_blur2);
		gl.activeTexture(gl.TEXTURE4); gl.bindTexture(gl.TEXTURE_2D, texture_blur3);
		gl.activeTexture(gl.TEXTURE5); gl.bindTexture(gl.TEXTURE_2D, texture_blur4);
		gl.activeTexture(gl.TEXTURE6); gl.bindTexture(gl.TEXTURE_2D, texture_blur5);
		gl.activeTexture(gl.TEXTURE7); gl.bindTexture(gl.TEXTURE_2D, texture_blur6);
		gl.activeTexture(gl.TEXTURE8); gl.bindTexture(gl.TEXTURE_2D, texture_noise_l);
		gl.activeTexture(gl.TEXTURE9); gl.bindTexture(gl.TEXTURE_2D, texture_noise_n);
		gl.activeTexture(gl.TEXTURE10); gl.bindTexture(gl.TEXTURE_2D, texture_fluid_v);
		gl.activeTexture(gl.TEXTURE11); gl.bindTexture(gl.TEXTURE_2D, texture_fluid_p);

		calculateBlurTexture();

		fluidInit(FBO_fluid_v);
		fluidInit(FBO_fluid_p);
		fluidInit(FBO_fluid_store);
		fluidInit(FBO_fluid_backbuffer);
		
		timer = setInterval(fr, 1000);
		time = new Date().getTime();

		anim();
	}
	
	function createAndLinkProgram(fsId){
		var program = gl.createProgram();
		gl.attachShader(program, getShader(gl, "shader-vs"));
		gl.attachShader(program, getShader(gl, fsId));
		gl.linkProgram(program);
		return program;
	}
	
	function createAndBindTexture(glPixels, scale, fbo, filter){
		var texture = gl.createTexture();
		gl.bindTexture(gl.TEXTURE_2D, texture);
		gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
		gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, sizeX/scale, sizeY/scale, 0, gl.RGBA, gl.UNSIGNED_BYTE, glPixels);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
		gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
		gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
		return texture;
	}
	
	function createAndBindSimulationTexture(glPixels, fbo){
		var texture = gl.createTexture();
		gl.bindTexture(gl.TEXTURE_2D, texture);
		gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
		gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, sizeX/simScale, sizeY/simScale, 0, gl.RGBA, gl.UNSIGNED_BYTE, glPixels);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S , gl.CLAMP_TO_EDGE);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T , gl.CLAMP_TO_EDGE);
		gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
		gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
		return texture;
	}
	
	function fluidInit(fbo){
		gl.viewport(0, 0, sizeX/simScale, sizeY/simScale);
		gl.useProgram(prog_fluid_init);
		gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
		gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
		gl.flush();
	}

	function setUniforms(program) {
		gl.uniform4f(gl.getUniformLocation(program, "rnd"), Math.random(), Math.random(), Math.random(), Math.random());
		gl.uniform4f(gl.getUniformLocation(program, "rainbow"), rainbowR, rainbowG, rainbowB, 1);
		gl.uniform2f(gl.getUniformLocation(program, "texSize"), sizeX, sizeY);
		gl.uniform2f(gl.getUniformLocation(program, "pixelSize"), 1. / sizeX, 1. / sizeY);
		gl.uniform2f(gl.getUniformLocation(program, "aspect"), Math.max(1, viewX / viewY), Math.max(1, viewY / viewX));
		gl.uniform2f(gl.getUniformLocation(program, "mouse"), mouseX, mouseY);
		gl.uniform2f(gl.getUniformLocation(program, "mouseV"), mouseDx, mouseDy);
		gl.uniform1f(gl.getUniformLocation(program, "fps"), fps);
		gl.uniform1f(gl.getUniformLocation(program, "time"), time);

		gl.uniform1i(gl.getUniformLocation(program, "sampler_prev"), 0);
		gl.uniform1i(gl.getUniformLocation(program, "sampler_prev_n"), 1);
		gl.uniform1i(gl.getUniformLocation(program, "sampler_blur"), 2);
		gl.uniform1i(gl.getUniformLocation(program, "sampler_blur2"), 3);
		gl.uniform1i(gl.getUniformLocation(program, "sampler_blur3"), 4);
		gl.uniform1i(gl.getUniformLocation(program, "sampler_blur4"), 5);
		gl.uniform1i(gl.getUniformLocation(program, "sampler_blur5"), 6);
		gl.uniform1i(gl.getUniformLocation(program, "sampler_blur6"), 7);
		gl.uniform1i(gl.getUniformLocation(program, "sampler_noise"), 8);
		gl.uniform1i(gl.getUniformLocation(program, "sampler_noise_n"), 9);
		gl.uniform1i(gl.getUniformLocation(program, "sampler_fluid"), 10);
		gl.uniform1i(gl.getUniformLocation(program, "sampler_fluid_p"), 11);
	}

	function calculateBlurTextures() {
		var texture_source = (it < 0 ) ? texture_main2_l : texture_main_l;
		calculateBlurTexture(texture_source, texture_blur, FBO_blur, texture_helper, FBO_helper, 1);
		calculateBlurTexture(texture_blur, texture_blur2, FBO_blur2, texture_helper2, FBO_helper2, 2);
		calculateBlurTexture(texture_blur2, texture_blur3, FBO_blur3, texture_helper3, FBO_helper3, 4);
		calculateBlurTexture(texture_blur3, texture_blur4, FBO_blur4, texture_helper4, FBO_helper4, 8);
		calculateBlurTexture(texture_blur4, texture_blur5, FBO_blur5, texture_helper5, FBO_helper5, 16);
		calculateBlurTexture(texture_blur5, texture_blur6, FBO_blur6, texture_helper6, FBO_helper6, 32);
	}
	
	function calculateBlurTexture(sourceTex, targetTex, targetFBO, helperTex, helperFBO, scale){
		// copy source
		gl.viewport(0, 0, sizeX / scale, sizeY / scale);
		gl.useProgram(prog_copy);
		gl.activeTexture(gl.TEXTURE0);
		gl.bindTexture(gl.TEXTURE_2D, sourceTex);
		gl.bindFramebuffer(gl.FRAMEBUFFER, targetFBO);
		gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
		gl.flush();

		// blur vertically
		gl.viewport(0, 0, sizeX / scale, sizeY / scale);
		gl.useProgram(prog_blur_vertical);
		gl.uniform2f(gl.getUniformLocation(prog_blur_vertical, "pixelSize"), scale / sizeX, scale / sizeY);
		gl.activeTexture(gl.TEXTURE0);
		gl.bindTexture(gl.TEXTURE_2D, targetTex);
		gl.bindFramebuffer(gl.FRAMEBUFFER, helperFBO);
		gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
		gl.flush();

		// blur horizontally
		gl.viewport(0, 0, sizeX / scale, sizeY / scale);
		gl.useProgram(prog_blur_horizontal);
		gl.uniform2f(gl.getUniformLocation(prog_blur_horizontal, "pixelSize"), scale / sizeX, scale / sizeY);
		gl.activeTexture(gl.TEXTURE0);
		gl.bindTexture(gl.TEXTURE_2D, helperTex);
		gl.bindFramebuffer(gl.FRAMEBUFFER, targetFBO);
		gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
		gl.flush();
	}

	function fluidSimulationStep() {
		addMouseMotion();
		advect();
		diffuse();
	}
	
	function addMouseMotion(){
		gl.viewport(0, 0, (sizeX/simScale), (sizeY/simScale));
		gl.useProgram(prog_fluid_add_mouse_motion);
		gl.activeTexture(gl.TEXTURE0);
		gl.bindTexture(gl.TEXTURE_2D, texture_fluid_v);
		gl.uniform2f(gl.getUniformLocation(prog_fluid_add_mouse_motion, "aspect"), Math.max(1, viewX / viewY), Math.max(1, viewY / viewX));
		gl.uniform2f(gl.getUniformLocation(prog_fluid_add_mouse_motion, "mouse"), mouseX, mouseY);
		gl.uniform2f(gl.getUniformLocation(prog_fluid_add_mouse_motion, "mouseV"), mouseDx, mouseDy);
		gl.uniform2f(gl.getUniformLocation(prog_fluid_add_mouse_motion, "pixelSize"), 1. / (sizeX/simScale), 1. / (sizeY/simScale));
		gl.uniform2f(gl.getUniformLocation(prog_fluid_add_mouse_motion, "texSize"), (sizeX/simScale), (sizeY/simScale));
		gl.bindFramebuffer(gl.FRAMEBUFFER, FBO_fluid_backbuffer);
		gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
		gl.flush();
	}
	
	function advect(){
		gl.viewport(0, 0, (sizeX/simScale), (sizeY/simScale));
		gl.useProgram(prog_fluid_advect);
		gl.activeTexture(gl.TEXTURE0);
		gl.bindTexture(gl.TEXTURE_2D, texture_fluid_backbuffer);
		gl.uniform2f(gl.getUniformLocation(prog_fluid_advect, "pixelSize"), 1. / (sizeX/simScale), 1. / (sizeY/simScale));
		gl.uniform2f(gl.getUniformLocation(prog_fluid_advect, "texSize"), (sizeX/simScale), (sizeY/simScale));
		gl.bindFramebuffer(gl.FRAMEBUFFER, FBO_fluid_v);
		gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
		gl.flush();
	}

	function diffuse(){
		for ( var i = 0; i < 8; i++) {
			gl.viewport(0, 0, (sizeX/simScale), (sizeY/simScale));
			gl.useProgram(prog_fluid_p);
			gl.activeTexture(gl.TEXTURE0);
			gl.bindTexture(gl.TEXTURE_2D, texture_fluid_v);
			gl.activeTexture(gl.TEXTURE1);
			gl.bindTexture(gl.TEXTURE_2D, texture_fluid_p);
			gl.uniform2f(gl.getUniformLocation(prog_fluid_p, "texSize"), (sizeX/simScale), (sizeY/simScale));
			gl.uniform2f(gl.getUniformLocation(prog_fluid_p, "pixelSize"), 1. / (sizeX/simScale), 1. / (sizeY/simScale));
			gl.uniform1i(gl.getUniformLocation(prog_fluid_p, "sampler_v"), 0);
			gl.uniform1i(gl.getUniformLocation(prog_fluid_p, "sampler_p"), 1);
			gl.bindFramebuffer(gl.FRAMEBUFFER, FBO_fluid_backbuffer);
			gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
			gl.flush();
	
			gl.viewport(0, 0, (sizeX/simScale), (sizeY/simScale));
			gl.useProgram(prog_fluid_p);
			gl.activeTexture(gl.TEXTURE0);
			gl.bindTexture(gl.TEXTURE_2D, texture_fluid_v);
			gl.activeTexture(gl.TEXTURE1);
			gl.bindTexture(gl.TEXTURE_2D, texture_fluid_backbuffer);
			gl.uniform2f(gl.getUniformLocation(prog_fluid_p, "texSize"), (sizeX/simScale), (sizeY/simScale));
			gl.uniform2f(gl.getUniformLocation(prog_fluid_p, "pixelSize"), 1. / (sizeX/simScale), 1. / (sizeY/simScale));
			gl.uniform1i(gl.getUniformLocation(prog_fluid_p, "sampler_v"), 0);
			gl.uniform1i(gl.getUniformLocation(prog_fluid_p, "sampler_p"), 1);
			gl.bindFramebuffer(gl.FRAMEBUFFER, FBO_fluid_p);
			gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
			gl.flush();
		}
		
		gl.viewport(0, 0, (sizeX/simScale), (sizeY/simScale));
		gl.useProgram(prog_fluid_div);
		gl.activeTexture(gl.TEXTURE0);
		gl.bindTexture(gl.TEXTURE_2D, texture_fluid_v);
		gl.activeTexture(gl.TEXTURE1);
		gl.bindTexture(gl.TEXTURE_2D, texture_fluid_p);
		gl.uniform2f(gl.getUniformLocation(prog_fluid_div, "texSize"), (sizeX/simScale), (sizeY/simScale));
		gl.uniform2f(gl.getUniformLocation(prog_fluid_div, "pixelSize"), 1. / (sizeX/simScale), 1. / (sizeY/simScale));
		gl.uniform1i(gl.getUniformLocation(prog_fluid_div, "sampler_v"), 0);
		gl.uniform1i(gl.getUniformLocation(prog_fluid_div, "sampler_p"), 1);
		gl.bindFramebuffer(gl.FRAMEBUFFER, FBO_fluid_v);
		gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
		gl.flush();

	}

	// main texture feedback warp
	function advance() {

		fluidSimulationStep();

		// texture warp step

		gl.viewport(0, 0, sizeX, sizeY);
		gl.useProgram(prog_advance);
		setUniforms(prog_advance);
		if (it > 0) {
			gl.activeTexture(gl.TEXTURE0);
			gl.bindTexture(gl.TEXTURE_2D, texture_main_l); // interpolated input
			gl.activeTexture(gl.TEXTURE1);
			gl.bindTexture(gl.TEXTURE_2D, texture_main_n); // "nearest" input
			gl.bindFramebuffer(gl.FRAMEBUFFER, FBO_main2); // write to buffer
		} else {
			gl.activeTexture(gl.TEXTURE0);
			gl.bindTexture(gl.TEXTURE_2D, texture_main2_l); // interpolated
			gl.activeTexture(gl.TEXTURE1);
			gl.bindTexture(gl.TEXTURE_2D, texture_main2_n); // "nearest"
			gl.bindFramebuffer(gl.FRAMEBUFFER, FBO_main); // write to buffer
		}
		gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
		gl.flush();

		calculateBlurTextures();

		it = -it;
	}

	function composite() {
		gl.viewport(0, 0, viewX, viewY);
		gl.useProgram(prog_composite);
		setUniforms(prog_composite);
		if (it < 0) {
			gl.activeTexture(gl.TEXTURE0);
			gl.bindTexture(gl.TEXTURE_2D, texture_main_l);
			gl.activeTexture(gl.TEXTURE1);
			gl.bindTexture(gl.TEXTURE_2D, texture_main_n);
		} else {
			gl.activeTexture(gl.TEXTURE0);
			gl.bindTexture(gl.TEXTURE_2D, texture_main2_l);
			gl.activeTexture(gl.TEXTURE1);
			gl.bindTexture(gl.TEXTURE_2D, texture_main2_n);
		}
		gl.bindFramebuffer(gl.FRAMEBUFFER, null);
		gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
		gl.flush();
		frames++;
	}

	var rainbowR, rainbowG, rainbowB, w = Math.PI * 2 / 3;

	function anim() {
		time = new Date().getTime();

		var t = time / 150;

		rainbowR = 0.5 + 0.5 * Math.sin(t);
		rainbowG = 0.5 + 0.5 * Math.sin(t + w);
		rainbowB = 0.5 + 0.5 * Math.sin(t - w);

		if (oldMouseX != 0 && oldMouseY != 0) {
			mouseDx = (mouseX - oldMouseX) * viewX;
			mouseDy = (mouseY - oldMouseY) * viewY;
		}

		if (!halted)
			advance();

		composite();

		setTimeout("requestAnimFrame(anim)", delay);

		oldMouseX = mouseX;
		oldMouseY = mouseY;

		frame++
	}

	function fr() { // updates every second
		document.getElementById("fps").textContent = frame;
		frame = 0;
	}
	
	var hidden = false;
	function hide(){
		hidden = !hidden;
		document.getElementById("desc").style.setProperty('visibility', hidden?'hidden':'visible' );
	}
